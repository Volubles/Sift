//! Bounded tag reads and accepted edits, owned by one background connection.

use super::{Change, Cursor, Filter, Membership, Mutation, Page, Store, Summary, Tag, TagId};
use crate::collections::{ReferenceSelection, SelectionOrigin};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};

#[derive(Clone, Debug)]
pub enum Target {
    Sources(Vec<PathBuf>),
    Selection(ReferenceSelection),
}
#[derive(Clone, Debug)]
pub enum Edit {
    Add(String),
    Remove(TagId),
}
#[derive(Debug)]
pub enum Event {
    Ready,
    StartupFailed(String),
    Suggestions {
        generation: u64,
        result: Result<Vec<Tag>, String>,
    },
    Status {
        generation: u64,
        result: Result<Membership, String>,
    },
    Summary {
        generation: u64,
        result: Result<Summary, String>,
    },
    Saved {
        operation: u64,
        result: Result<Vec<PathBuf>, String>,
    },
    ManagementPage {
        generation: u64,
        result: Result<Page, String>,
    },
    Managed {
        operation: u64,
        result: Result<Change, String>,
    },
}
enum Command {
    Suggestions {
        generation: u64,
        prefix: String,
    },
    Status {
        generation: u64,
        sources: Vec<PathBuf>,
        filter: Filter,
    },
    Summary {
        generation: u64,
        target: Target,
    },
    Edit {
        operation: u64,
        target: Target,
        edit: Edit,
    },
    ManagementPage {
        generation: u64,
        prefix: String,
        after: Option<Cursor>,
    },
    Manage {
        operation: u64,
        mutation: Mutation,
    },
}
#[derive(Default)]
struct Generations {
    suggestions: AtomicU64,
    status: AtomicU64,
    summary: AtomicU64,
    management: AtomicU64,
}

pub struct Worker {
    commands: Option<SyncSender<Command>>,
    events: Option<Receiver<Event>>,
    reads: Arc<Generations>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(catalog_path: PathBuf, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (commands, jobs) = mpsc::sync_channel(4);
        let (sender, events) = mpsc::sync_channel(4);
        let reads = Arc::new(Generations::default());
        let generations = reads.clone();
        let thread = thread::Builder::new()
            .name("sift-tags".into())
            .spawn(move || {
                let mut store = match Store::open(&super::data_path(&catalog_path)) {
                    Ok(store) => store,
                    Err(error) => {
                        if sender.send(Event::StartupFailed(error.to_string())).is_ok() {
                            wake();
                        }
                        return;
                    }
                };
                let mut catalog = None;
                let mut saved = None;
                let mut listening = sender.send(Event::Ready).is_ok();
                if listening {
                    wake();
                }
                while let Ok(command) = jobs.recv() {
                    let stale = match &command {
                        Command::Suggestions { generation, .. } => {
                            *generation < generations.suggestions.load(Ordering::Acquire)
                        }
                        Command::Status { generation, .. } => {
                            *generation < generations.status.load(Ordering::Acquire)
                        }
                        Command::Summary { generation, .. } => {
                            *generation < generations.summary.load(Ordering::Acquire)
                        }
                        Command::ManagementPage { generation, .. } => {
                            *generation < generations.management.load(Ordering::Acquire)
                        }
                        Command::Edit { .. } | Command::Manage { .. } => false,
                    };
                    if !matches!(command, Command::Edit { .. } | Command::Manage { .. })
                        && (!listening || stale)
                    {
                        if listening {
                            wake();
                        }
                        continue;
                    }
                    let event = match command {
                        Command::ManagementPage {
                            generation,
                            prefix,
                            after,
                        } => Event::ManagementPage {
                            generation,
                            result: store
                                .tag_page(&prefix, after.as_ref())
                                .map_err(|e| e.to_string()),
                        },
                        Command::Manage {
                            operation,
                            mutation,
                        } => {
                            let result = store.manage(&mutation).map_err(|e| e.to_string());
                            if let Err(error) = &result {
                                eprintln!("Tag management save {operation} failed: {error}");
                            }
                            Event::Managed { operation, result }
                        }
                        Command::Suggestions { generation, prefix } => Event::Suggestions {
                            generation,
                            result: store.suggestions(&prefix).map_err(|e| e.to_string()),
                        },
                        Command::Status {
                            generation,
                            sources,
                            filter,
                        } => Event::Status {
                            generation,
                            result: store
                                .status(&sources)
                                .and_then(|sources| {
                                    Ok(Membership {
                                        sources,
                                        filter_tags: store.definitions(filter.ids())?,
                                    })
                                })
                                .map_err(|e| e.to_string()),
                        },
                        Command::Summary { generation, target } => Event::Summary {
                            generation,
                            result: resolve(&catalog_path, &mut catalog, &mut saved, &target)
                                .and_then(|sources| {
                                    store.summary(&sources).map_err(|e| e.to_string())
                                }),
                        },
                        Command::Edit {
                            operation,
                            target,
                            edit,
                        } => {
                            let result = resolve(&catalog_path, &mut catalog, &mut saved, &target)
                                .and_then(|sources| {
                                    match edit {
                                        Edit::Add(name) => store.add(&name, &sources).map(|_| ()),
                                        Edit::Remove(id) => store.remove(id, &sources),
                                    }
                                    .map_err(|e| e.to_string())?;
                                    Ok(sources)
                                });
                            if let Err(error) = &result {
                                eprintln!("Tag save {operation} failed: {error}");
                            }
                            Event::Saved { operation, result }
                        }
                    };
                    if listening {
                        listening = sender.send(event).is_ok();
                        if listening {
                            wake();
                        }
                    }
                }
            })?;
        Ok(Self {
            commands: Some(commands),
            events: Some(events),
            reads,
            thread: Some(thread),
        })
    }
    pub fn poll(&self) -> Option<Event> {
        self.events.as_ref()?.try_recv().ok()
    }
    pub fn suggestions(&self, generation: u64, prefix: String) -> bool {
        self.reads
            .suggestions
            .fetch_max(generation, Ordering::AcqRel);
        self.send(Command::Suggestions { generation, prefix })
    }
    pub fn status(&self, generation: u64, sources: Vec<PathBuf>, filter: Filter) -> bool {
        self.reads.status.fetch_max(generation, Ordering::AcqRel);
        self.send(Command::Status {
            generation,
            sources,
            filter,
        })
    }
    pub fn summary(&self, generation: u64, target: Target) -> bool {
        self.reads.summary.fetch_max(generation, Ordering::AcqRel);
        self.send(Command::Summary { generation, target })
    }
    pub fn edit(&self, operation: u64, target: Target, edit: Edit) -> Result<(), String> {
        self.submit(Command::Edit {
            operation,
            target,
            edit,
        })
    }
    pub fn management_page(&self, generation: u64, prefix: String, after: Option<Cursor>) -> bool {
        self.reads
            .management
            .fetch_max(generation, Ordering::AcqRel);
        self.send(Command::ManagementPage {
            generation,
            prefix,
            after,
        })
    }
    pub fn manage(&self, operation: u64, mutation: Mutation) -> Result<(), String> {
        self.submit(Command::Manage {
            operation,
            mutation,
        })
    }
    fn submit(&self, command: Command) -> Result<(), String> {
        match self
            .commands
            .as_ref()
            .ok_or("Tag storage is unavailable")?
            .try_send(command)
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("Tag storage is busy; retry the pending edit".into()),
            Err(TrySendError::Disconnected(_)) => {
                Err("Tag storage has stopped; reopen Sift to retry".into())
            }
        }
    }
    fn send(&self, command: Command) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.try_send(command).is_ok())
    }
}
fn resolve(
    path: &std::path::Path,
    catalog: &mut Option<crate::catalog::Catalog>,
    saved: &mut Option<crate::collections::Store>,
    target: &Target,
) -> Result<Vec<PathBuf>, String> {
    match target {
        Target::Sources(sources) => {
            super::encode_sources(sources, super::Batch::Edit).map_err(|e| e.to_string())?;
            Ok(sources.clone())
        }
        Target::Selection(selection) => {
            let references = match &selection.origin {
                SelectionOrigin::Library { query, scope } => {
                    if catalog.is_none() {
                        *catalog = Some(
                            crate::catalog::Catalog::open_reader(path)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    catalog
                        .as_mut()
                        .ok_or("Catalog unavailable")?
                        .selected_references_ordered(
                            query,
                            scope,
                            selection.order,
                            &selection.sources,
                        )
                        .map_err(|e| e.to_string())?
                }
                _ => {
                    if saved.is_none() {
                        *saved = Some(
                            crate::collections::Store::open_reader(&crate::collections::data_path(
                                path,
                            ))
                            .map_err(|e| e.to_string())?,
                        );
                    }
                    saved
                        .as_mut()
                        .ok_or("Saved selections unavailable")?
                        .selected_references(selection)
                        .map_err(|e| e.to_string())?
                }
            };
            Ok(references.into_iter().map(|r| r.source).collect())
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.commands.take();
        self.events.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("Tag storage worker panicked during shutdown");
        }
    }
}
