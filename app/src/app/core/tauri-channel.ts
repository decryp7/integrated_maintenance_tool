import { Channel } from '@tauri-apps/api/core';

/**
 * A Tauri IPC channel whose messages go to `onMessage` (pass it as a command argument; Rust receives a
 * `tauri::ipc::Channel`). Kept apart from the backends so they stay testable without the Tauri runtime.
 */
export function tauriChannel<T>(onMessage: (message: T) => void): Channel<T> {
  const channel = new Channel<T>();
  channel.onmessage = onMessage;
  return channel;
}

export type ChannelFactory = <T>(onMessage: (message: T) => void) => unknown;
