# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`matrix-ui-serializable` is a Rust library that wraps `matrix-sdk` / `matrix-sdk-ui` in a still-higher-level, **frontend-agnostic** abstraction. It exposes serializable structs (`RoomsList`, `RoomScreen`, profiles, etc.) representing reactive UI state. It is not an app — it is consumed by an "adapter" (e.g. [tauri-plugin-matrix-svelte](https://github.com/IT-ess/tauri-plugin-matrix-svelte)) that bridges the serialized state to a concrete frontend. The design follows MVVM: this lib + adapter are Model/ViewModel, the frontend is the View.

Much of the logic is a port/adaptation of [Robrix](https://github.com/project-robius/robrix).

## Commands

```bash
cargo build                 # build the library
cargo clippy --all-targets  # lint (treat warnings as the bar to clear)
cargo fmt                   # format
cargo doc --open            # the public API is the contract for adapters; docs matter
```

There is currently **no test suite** (no `#[test]`/`#[tokio::test]` in `src/`). `cargo mutants` artifacts are gitignored, suggesting mutation testing has been used ad hoc. Verification is mostly through downstream adapters/examples.

## Module visibility convention

Only `commands` and `models` are `pub`. Everything else (`account`, `events`, `init`, `room`, `stores`, `user`, `utils`) is `pub(crate)`. **The public API surface that adapters depend on lives in `src/commands.rs`, `src/models/`, and `src/lib.rs`** — changing signatures there is a breaking change for adapters. Internal modules can be refactored freely.

## The three communication channels (adapter <-> lib)

Everything an adapter does flows through one of these three mechanisms; understanding them is the key to the codebase:

1. **State updates (lib -> frontend).** The adapter implements the `StateUpdater` trait (`src/models/state_updater.rs`) and passes it in `LibConfig`. Its `update_*` methods (`update_rooms_list`, `update_room`, `update_login_state`, `persist_login_session`, ...) receive the backend Rust structs (which all derive `Serialize`) and are responsible for translating/serializing them into the frontend's state. The lib calls these whenever state changes.

2. **Commands (frontend -> lib, with a response).** Free functions in `src/commands.rs` the adapter exposes to its frontend (login, media upload, profile edits, room creation, fetching previews, etc.).

3. **Events.**
   - *Outgoing* (`EmitEvent` in `src/models/events.rs`): broadcast via a tokio `broadcast` channel. `init()` returns the `Receiver`; the adapter forwards events to the frontend.
   - *Incoming* (`EventReceivers` in `src/lib.rs`): tokio `mpsc` receivers the adapter feeds (verification responses, active-room changes, login payloads, OAuth deeplinks).

## Initialization & runtime model

`init(LibConfig)` (`src/lib.rs`) is the single entry point; it returns `broadcast::Receiver<EmitEvent>`. `LibConfig::new` takes the `StateUpdater`, `EventReceivers`, an optional serialized session (restore vs. fresh login), the app data dir (`PathBuf` for the Matrix sqlite DB), and OAuth client/redirect URIs.

The runtime is built on **global singletons** (`src/init/singletons.rs`) initialized once during `init` — notably `CLIENT`, `CURRENT_USER_ID`, `SYNC_SERVICE`, `REQUEST_SENDER`, `EVENT_BRIDGE`, `APP_DATA_DIR`, and a `GLOBAL_BROADCASTER` for `UIUpdateMessage::RefreshUI`. Because state is global, this lib is effectively a single-client singleton per process.

Login supports two flows decided by `check_homeserver_auth_type`: native Matrix password auth and OAuth (`src/init/oauth.rs`, `login.rs`). A `TEMP_CLIENT` is held during login because the user can change homeserver before authenticating. Token refresh persistence is wired via `setup_token_background_save` + `persist_refreshed_session`.

## Two-tier async worker / queue pattern (important)

Work is decoupled from UI rendering through two layers — when adding a feature, follow this pattern rather than calling the SDK directly from a command:

- **`MatrixRequest` async worker** (`src/models/async_requests.rs` + `async_worker` in `src/init/workers.rs`). UI/command code calls `submit_async_request(MatrixRequest::...)`, which sends over `REQUEST_SENDER` to a single async worker. The worker matches the request and usually spawns a task that talks to the SDK (paginate, edit, react, redact, join/leave, fetch profiles/members/previews, send messages, etc.). To add an operation: add a variant to `MatrixRequest`, handle it in `async_worker`, and call `submit_async_request`.

- **`SegQueue` enqueue/process pattern.** Several subsystems use a global `crossbeam_queue::SegQueue` with an `enqueue_*` producer and a `process_*` consumer drained on UI refresh: `rooms_list` (`RoomsListUpdate`), `preview` (`RoomPreviewUpdate`), `notifications` (toast), `user/user_profile` (`UserProfileUpdate`). Producers can run from any task; the consumer runs in `ui_worker`.

The **`ui_worker`** (`src/init/workers.rs`) owns the single `RoomsList` (behind a `Mutex`), listens for active-room changes, and on each debounced `RefreshUI` broadcast (debounced ~200ms via `debounce_broadcast`) drains the queues and pushes state to the adapter through the `StateUpdater`. `async_main_loop` starts the sync (`src/init/sync.rs`) and the `ui_worker`.

## Timelines

`TimelineKind` (`src/events/timeline.rs`) distinguishes `MainRoom { room_id }` from `Thread { room_id, thread_root_event_id }` and is the key for most per-timeline state and requests. Each open timeline has its own unbounded `crossbeam_channel` of `TimelineUpdate`s, created when a room is joined or a thread is opened. `RoomScreen` (`src/room/room_screen.rs`) holds a timeline's UI state and `process_timeline_updates()` applies pending updates. Frontend-facing event DTOs live under `src/room/frontend_events/` (msg-like events, state events, virtual events, thread summaries, timeline item IDs).

## Other notable areas

- `src/account/` — device verification, recovery, key backup.
- `src/room/` — `rooms_list`, `joined_room`/`invited_room`, `room_screen`, `preview`, `room_filter`, `tags`, `notifications` (incl. push via Sygnal).
- `src/models/matrix_uri.rs` + `events/event_preview.rs` — matrix.to URI / pill resolution and message previews.
- `#![recursion_limit = "256"]` is set in `lib.rs` (needed for the SDK's generated types).
- Edition 2024; `matrix-sdk` is pinned to `0.18.0` with `default-features = false` — SDK minor bumps are routinely breaking, so expect to adapt call sites when bumping (see recent commits).

## Errors

The crate's `Error` (`src/lib.rs`) wraps `io`, `anyhow`, and `matrix_sdk::Error`, and implements `Serialize` (as its `Display` string) so command errors can cross to the frontend. Public commands return `crate::Result<T>`; internal code freely uses `anyhow`.
