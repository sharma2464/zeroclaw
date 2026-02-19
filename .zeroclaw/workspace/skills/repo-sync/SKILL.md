# Skill: Repo Sync

This skill enables the AI to safely synchronize your local ZeroClaw fork with the upstream repository (`origin/main`).

## 🛠️ Capabilities
- **Safely Fetches Updates:** Pulls metadata from the remote without modifying local files.
- **Rebase & Validate:** Replays local commits (like Termux patches) on top of upstream changes and validates with `cargo build`.
- **Atomic Failure:** If the new upstream code breaks the build in the local environment, it automatically aborts the update to keep the workspace functional.

## 📖 Instructions
- Use `sync_upstream` when the user asks to "update", "pull changes", or "sync with origin".
- Always confirm the current branch is `main` before running a sync.
- If the sync fails with conflicts, report the specific files that need manual resolution.
