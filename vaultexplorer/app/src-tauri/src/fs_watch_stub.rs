//! `fs_watch` on a phone: nothing to watch.
//!
//! The real module (see `fs_watch.rs`) live-updates the file list when
//! something outside the app changes the folder being browsed. On Android
//! that does not apply -- files arrive through the system picker or this
//! app's own operations, both of which already refresh -- and its `notify`
//! dependency is desktop-only anyway.
//!
//! A stub with the same shape rather than a second, shorter command list:
//! one list is easier to keep honest, and `fs_watch_set` quietly doing
//! nothing is better than an error the frontend would have to special-case
//! on one platform.

use tauri::{AppHandle, State};

#[derive(Default)]
pub struct FsWatchState;

#[tauri::command]
pub fn fs_watch_set(_app: AppHandle, _state: State<FsWatchState>, _path: Option<String>) {}
