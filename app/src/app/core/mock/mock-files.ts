import { FileKind, FileOpened, OpenFileRequest } from '../api.types';

/** Archive member separator of source paths (`x.zip!/a.log`), as the engine writes them. */
const ENTRY_SEPARATOR = '!/';

/** Extensions the engine never opens with their default program (they could run). Mirrors `opener::RUNS_ON_OPEN` in part. */
const RUNS_ON_OPEN = new Set(['exe', 'com', 'bat', 'cmd', 'ps1', 'vbs', 'js', 'msi', 'scr', 'lnk', 'reg', 'sh', 'command', 'app', 'desktop']);

/** How a mock source is stored, from its path (the engine knows it from the import). */
export function mockFileKind(path: string): FileKind {
  if (path.includes(ENTRY_SEPARATOR)) return 'archiveMember';
  if (/\.gz$/i.test(path)) return 'compressed';
  if (/\.(evtx|etl)$/i.test(path)) return 'binary';
  return 'text';
}

/** The mock of `open_source_file`: the engine's checks, without opening anything (the browser has no file system). */
export function mockOpenSourceFile(sources: readonly { id: number; path: string; name: string }[], request: OpenFileRequest): FileOpened {
  const source = sources.find((s) => s.id === request.sourceId);
  if (!source) throw new Error('that file is no longer open in this window');
  const kind = mockFileKind(source.path);
  const archive = source.path.split(ENTRY_SEPARATOR)[0] ?? source.path;
  if (request.action === 'reveal') return { with: 'fileManager', path: archive };
  if (kind === 'archiveMember') throw new Error(`${source.name} is inside the archive ${archive.split(/[\\/]/).pop()}: extract it to open it, or use Show in folder`);
  if (request.action === 'openAtLine') {
    if (kind !== 'text') throw new Error(`${source.name} is not a text file, so there is no line to jump to: use Open file`);
    if (!request.line || request.line < 1) throw new Error('this event has no line number: use Open file');
    return { with: 'editor', path: source.path };
  }
  const ext = (source.path.split(/[\\/]/).pop() ?? '').split('.').slice(1).pop()?.toLowerCase() ?? '';
  if (RUNS_ON_OPEN.has(ext)) throw new Error(`Debrief does not open ${source.name} with its default program, because that could run it: use Show in folder or Open at line`);
  return { with: 'defaultApp', path: source.path };
}
