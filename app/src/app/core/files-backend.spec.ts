import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, tinyBackend } from '../../testing/helpers';
import { FilesBackend, hasFiles, TauriFilesBackend, UnsupportedFilesBackend } from './files-backend';
import { mockFileKind, mockOpenSourceFile } from './mock/mock-files';

describe('FilesBackend', () => {
  it('sends the source id, action and line to open_source_file', async () => {
    const call = vi.fn().mockResolvedValue({ with: 'editor', path: '/a.log' });
    const backend = new TauriFilesBackend(call as never);
    expect(await backend.openSourceFile({ sourceId: 2, action: 'openAtLine', line: 9 })).toEqual({ with: 'editor', path: '/a.log' });
    expect(call).toHaveBeenCalledWith('open_source_file', { request: { sourceId: 2, action: 'openAtLine', line: 9 } });
  });

  it('is the mock workspace in a browser, and refuses without one', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    expect(hasFiles(TestBed.inject(FilesBackend))).toBe(true);
    expect(hasFiles(tinyBackend())).toBe(true);
    expect(hasFiles({})).toBe(false);
    expect(hasFiles(null)).toBe(false);
    await expect(new UnsupportedFilesBackend().openSourceFile()).rejects.toMatch(/cannot open files/);
  });
});

describe('mock open_source_file', () => {
  const sources = [
    { id: 0, path: '/l/app.log', name: 'app.log' },
    { id: 1, path: '/l/b.zip!/x/in.log', name: 'b.zip/x/in.log' },
    { id: 2, path: '/l/old.log.gz', name: 'old.log.gz' },
    { id: 3, path: '/l/run.bat', name: 'run.bat' },
  ];

  it('derives the file kind from the path', () => {
    expect(['/a.log', '/a.zip!/b', '/a.log.GZ', '/s.evtx'].map(mockFileKind)).toEqual(['text', 'archiveMember', 'compressed', 'binary']);
  });

  it('mimics the engine checks', () => {
    expect(mockOpenSourceFile(sources, { sourceId: 0, action: 'openAtLine', line: 3 })).toEqual({ with: 'editor', path: '/l/app.log' });
    expect(mockOpenSourceFile(sources, { sourceId: 0, action: 'open' })).toEqual({ with: 'defaultApp', path: '/l/app.log' });
    expect(mockOpenSourceFile(sources, { sourceId: 1, action: 'reveal' })).toEqual({ with: 'fileManager', path: '/l/b.zip' });
    expect(() => mockOpenSourceFile(sources, { sourceId: 1, action: 'open' })).toThrow(/inside the archive b\.zip/);
    expect(() => mockOpenSourceFile(sources, { sourceId: 2, action: 'openAtLine', line: 1 })).toThrow(/not a text file/);
    expect(() => mockOpenSourceFile(sources, { sourceId: 0, action: 'openAtLine' })).toThrow(/no line number/);
    expect(() => mockOpenSourceFile(sources, { sourceId: 3, action: 'open' })).toThrow(/could run it/);
    expect(() => mockOpenSourceFile(sources, { sourceId: 9, action: 'open' })).toThrow(/no longer open/);
  });

  it('rejects with a message through the mock backend', async () => {
    await expect(tinyBackend().openSourceFile({ sourceId: 42, action: 'reveal' })).rejects.toMatch(/no longer open/);
    expect((await tinyBackend().workspaceSummary()).sources.map((s) => s.fileKind)).toEqual(['text', 'text']);
  });
});
