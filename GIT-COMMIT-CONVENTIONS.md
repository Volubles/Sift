# Git and checkpoint commits

## Principle

Benjamin wants frequent, natural commits that show the project developing step
by step. Commit where a developer would reasonably save a working checkpoint:
a coherent, understandable, verified part of the work. Do not wait for a whole
version to be finished or request separate approval for each local commit.

A commit needs one clear purpose and must make sense without the conversation.
Implementation, necessary tests, and related documentation often belong together.
Size follows the actual boundary, not an arbitrary file or line count.
Do not commit broken intermediate states or manufacture commits per file.

## Format

Follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>(<optional scope>): <short English description>

<optional explanation of why, behavior, and validation>

<optional breaking-change or issue footer>
```

Use lowercase types/scopes and a short, concrete English subject. Scope identifies
the responsibility, such as `library`, `search`, `audio`, `projects`, `export`,
`ui`, `storage`, or `repo`. Omit it when it adds no useful information.

| Type | Meaning |
| --- | --- |
| `feat` | New usable functionality. |
| `fix` | Correction of incorrect behavior. |
| `perf` | Behavior-preserving improvement with relevant performance evidence. |
| `refactor` | Structural change preserving behavior. |
| `test` | Meaningful verification without changing product behavior. |
| `docs` | Documentation and specifications. |
| `build` / `ci` | Build system / automated checks. |
| `chore` | Repository or tooling maintenance. |
| `style` | Formatting only, without behavior changes. |
| `revert` | Intentional reversal of an earlier change. |

Mark actual incompatible changes with `!` and/or a `BREAKING CHANGE:` footer.
Explain migration implications. A schema change is not automatically breaking
if existing data migrates safely. A commit is not a release.

## Good checkpoints

Examples for a substantial task; split only where the boundary is meaningful:

```text
feat(library): persist registered sample roots
feat(library): index changed audio metadata in bounded batches
feat(search): query indexed sample names and tags
feat(ui): browse paginated sample search results
fix(audio): discard decoded buffers after a newer selection
feat(projects): save sample references with project ordering
feat(export): copy selected samples without replacing existing files
```

Plan natural checkpoints before substantial implementation and adapt as evidence
changes. A persistence foundation can stand alone before UI uses it when its
contract and tests are complete. A change that requires caller updates belongs
with those updates.

## Before every commit

1. Check the branch and worktree; distinguish your changes from existing work.
2. Run appropriate checks and inspect the resulting behavior.
3. Stage explicit paths or reviewed hunks. Do not use blind `git add .`.
4. Read `git diff --cached`, check `git diff --cached --check` and
   `git diff --cached --name-only`. Exclude private data and unrelated files.
5. Commit, confirm its contents, and report the hash and subject.

Resolve uncertainties before committing. Status updates belong in their relevant
checkpoint, not an endless chain of housekeeping commits. Do not add hidden hooks
or installation steps requiring manual setup before a normal commit works.

## Branches and history

- `main` is the local baseline. Check the worktree before creating a branch.
- Substantial independent work normally uses `codex/<short-purpose>` from the
  intended current base. Reuse a suitable active branch. Related small changes
  do not require a new branch per checkpoint.
- If a remote is introduced, inspect/fetch the intended base before creating an
  independent branch. Investigate unexpected divergence before integration.
- A local commit does not automatically authorize push, a PR, or a release.
- Do not use reset, clean, rebase, force-push, stash, or amend to hide or relocate
  unrelated/shared work. New commits are the default.
- Do not fabricate authors, dates, verification, or history. Preserve useful
  checkpoints and correct published history with a new commit.

## What belongs in Git?

Code, the application's `Cargo.lock`, toolchain/build configuration, small
rights-cleared test fixtures, project documentation, and this Sift guide.
Private libraries, exports, databases, logs, raw measurements, credentials,
IDE state, and build output stay outside Git. Ignore rules help; the staged diff
is still the final check.
