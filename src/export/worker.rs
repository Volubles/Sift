use super::{Outcome, Plan, PreviewRow, Request, Runtime};
use crate::{
    collections::{Reference, ReferenceSelection, SelectionOrigin},
    library::Cancellation,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::AtomicU64,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};

#[derive(Clone, Debug)]
pub enum Input {
    Selection(ReferenceSelection),
    References(Vec<Reference>),
}
pub enum Event {
    Loaded {
        revision: u64,
        result: Result<Vec<Reference>, String>,
    },
    Reviewed {
        revision: u64,
        result: Result<Vec<PreviewRow>, String>,
    },
    Started {
        revision: u64,
        index: usize,
    },
    Item {
        revision: u64,
        index: usize,
        outcome: Outcome,
    },
    Finished {
        revision: u64,
    },
    Failed {
        revision: u64,
        error: String,
    },
}
enum Command {
    Discard,
    Load {
        revision: u64,
        input: Input,
        token: Cancellation,
    },
    Review {
        revision: u64,
        request: Request,
        token: Cancellation,
    },
    Run {
        revision: u64,
        token: Cancellation,
    },
}
pub struct Worker {
    commands: Option<SyncSender<Command>>,
    events: Option<Receiver<Event>>,
    thread: Option<JoinHandle<()>>,
    cancellation: Cancellation,
    pub progress: Arc<AtomicU64>,
}
impl Worker {
    pub fn start(catalog: PathBuf, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        Self::start_with_runtime(catalog, None, wake)
    }
    pub fn start_with_runtime(
        catalog: PathBuf,
        runtime: Option<PathBuf>,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let (commands, jobs) = mpsc::sync_channel(1);
        let (sender, events) = mpsc::sync_channel(4);
        let progress = Arc::new(AtomicU64::new(0));
        let advanced = progress.clone();
        let thread = thread::Builder::new()
            .name("sift-export".into())
            .spawn(move || {
                let mut plan: Option<(u64, Plan)> = None;
                let mut runtime = runtime.map(Runtime::new);
                let mut verified = false;
                let publish = |event| {
                    let sent = sender.send(event).is_ok();
                    if sent {
                        wake();
                    }
                    sent
                };
                while let Ok(command) = jobs.recv() {
                    match command {
                        Command::Discard => plan = None,
                        Command::Load {
                            revision,
                            input,
                            token,
                        } => {
                            plan = None;
                            let result = if token.is_cancelled() {
                                Err("Export cancelled".into())
                            } else {
                                resolve(&catalog, input)
                            };
                            if !publish(Event::Loaded { revision, result }) {
                                break;
                            }
                        }
                        Command::Review {
                            revision,
                            request,
                            token,
                        } => {
                            plan = None;
                            let result = super::prepare(request, &token)
                                .and_then(|prepared| {
                                    if prepared.needs_encoder() && !verified {
                                        if runtime.is_none() {
                                            runtime = Some(Runtime::bundled()?);
                                        }
                                        runtime
                                            .as_ref()
                                            .ok_or_else(|| {
                                                super::Error::Invalid(
                                                    "Export tools are unavailable".into(),
                                                )
                                            })?
                                            .verify(&token)?;
                                        verified = true;
                                    }
                                    if token.is_cancelled() {
                                        return Err(super::Error::Cancelled);
                                    }
                                    let rows = prepared.rows();
                                    plan = Some((revision, prepared));
                                    Ok(rows)
                                })
                                .map_err(|e| e.to_string());
                            if let Err(error) = &result
                                && !token.is_cancelled()
                            {
                                eprintln!("Export review {revision} failed: {error}");
                            }
                            if !publish(Event::Reviewed { revision, result }) {
                                break;
                            }
                        }
                        Command::Run { revision, token } => {
                            match plan.take().filter(|(id, _)| *id == revision) {
                                Some((_, plan)) => {
                                    plan.execute(
                                        runtime.as_ref(),
                                        &token,
                                        &advanced,
                                        |index, outcome| {
                                            let event = match outcome {
                                                None => Event::Started { revision, index },
                                                Some(outcome) => Event::Item {
                                                    revision,
                                                    index,
                                                    outcome: outcome.clone(),
                                                },
                                            };
                                            if !publish(event) {
                                                token.cancel();
                                            }
                                        },
                                    );
                                    if !publish(Event::Finished { revision }) {
                                        break;
                                    }
                                }
                                None => {
                                    if !publish(Event::Failed {
                                        revision,
                                        error: "Export settings changed; review the files again"
                                            .into(),
                                    }) {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            commands: Some(commands),
            events: Some(events),
            thread: Some(thread),
            cancellation: Cancellation::default(),
            progress,
        })
    }
    pub fn poll(&self) -> Option<Event> {
        self.events.as_ref()?.try_recv().ok()
    }
    fn token(&mut self) -> Cancellation {
        self.cancellation.cancel();
        self.cancellation = Cancellation::default();
        self.cancellation.clone()
    }
    pub fn load(&mut self, revision: u64, input: Input) -> Result<(), String> {
        let token = self.token();
        self.send(Command::Load {
            revision,
            input,
            token,
        })
    }
    pub fn review(&mut self, revision: u64, request: Request) -> Result<(), String> {
        let token = self.token();
        self.send(Command::Review {
            revision,
            request,
            token,
        })
    }
    pub fn run(&mut self, revision: u64) -> Result<(), String> {
        let token = self.token();
        self.send(Command::Run { revision, token })
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
        if let Some(sender) = &self.commands {
            let notified = match sender.try_send(Command::Discard) {
                Ok(()) => true,
                // Every queued command either clears or consumes the retained plan;
                // a full queue already guarantees the same resource release.
                Err(mpsc::TrySendError::Full(_)) => true,
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            };
            if !notified {
                eprintln!("Export cancellation reached a stopped worker");
            }
        }
    }
    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .as_ref()
            .ok_or("Export worker is unavailable")?
            .try_send(command)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "Export worker is busy; retry the action".into(),
                mpsc::TrySendError::Disconnected(_) => "Export worker stopped; reopen Sift".into(),
            })
    }
}
fn resolve(catalog: &std::path::Path, input: Input) -> Result<Vec<Reference>, String> {
    let references = match input {
        Input::References(references) => references,
        Input::Selection(selection) => match selection.origin {
            SelectionOrigin::Library { query, scope } => {
                crate::catalog::Catalog::open_reader(catalog)
                    .and_then(|mut reader| {
                        reader.selected_references_ordered(
                            &query,
                            &scope,
                            selection.order,
                            &selection.sources,
                        )
                    })
                    .map_err(|e| e.to_string())?
            }
            _ => crate::collections::Store::open_reader(&crate::collections::data_path(catalog))
                .and_then(|mut store| store.selected_references(&selection))
                .map_err(|e| e.to_string())?,
        },
    };
    if references.is_empty() || references.len() > crate::search::MAX_ACTION_SOUNDS {
        return Err("Export between 1 and 128 sounds at a time".into());
    }
    Ok(references)
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.commands.take();
        self.events.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("Export worker panicked during shutdown");
        }
    }
}
