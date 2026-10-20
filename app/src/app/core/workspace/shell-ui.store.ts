import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';

interface ShellUiState {
  sidebarOpen: boolean;
  /** Files are dragged over the window (the drop overlay shows). */
  dropHover: boolean;
  /** Bumped to ask the search box to take focus. */
  focusSearchNonce: number;
}

/** Transient shell flags: the sidebar, the drop overlay and focus requests for the search box. */
export const ShellUiStore = signalStore(
  { providedIn: 'root' },
  withState<ShellUiState>({ sidebarOpen: true, dropHover: false, focusSearchNonce: 0 }),
  withMethods((store) => ({
    toggleSidebar: () => patchState(store, (s) => ({ sidebarOpen: !s.sidebarOpen })),
    setDropHover: (dropHover: boolean) => patchState(store, { dropHover }),
    focusSearch: () => patchState(store, (s) => ({ focusSearchNonce: s.focusSearchNonce + 1 })),
  })),
);
