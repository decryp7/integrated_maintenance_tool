/**
 * The promise Debrief makes about the user's files, shown on the welcome screen, in About and on the source list.
 * Keep it true: Debrief reads opened files and folders and never writes to them; it writes only where the user
 * explicitly saves or exports (and never replaces an existing file without asking).
 */
export const READ_ONLY_PROMISE = 'Debrief never modifies your files.';
export const READ_ONLY_DETAIL =
  'Opened logs and folders are only read. Profile cleaning steps, masks and markers change what Debrief shows, not the files. Debrief writes only where you explicitly save or export, and asks before replacing an existing file.';
