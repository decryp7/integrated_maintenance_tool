/** Plausible data for the scripted AI mock: an 8 GB VRAM laptop, a small catalog, proposals and answers. */
import {
  CatalogModel,
  ExtractorsProposal,
  Hardware,
  InstalledModel,
  MarkersProposal,
  PipelineProposal,
  ParserProposal,
  ProjectorInfo,
  Recommendation,
  Recommendations,
  RegexProposal,
} from '../../ai/ai.types';
import { generateDemo } from './mock-data';
import { MOCK_PROFILE_TOML } from './mock-profile';

const GB = 1024 ** 3;

export const MOCK_HARDWARE: Hardware = {
  os: 'windows',
  arch: 'x86_64',
  cpuCores: 16,
  totalRamGb: 32,
  availableRamGb: 21.5,
  gpus: [{ name: 'NVIDIA GeForce RTX 4060 Laptop GPU', vramGb: 8, unified: false }],
};

const MB = 1024 ** 2;

function model(id: string, name: string, paramsB: number, sizeGb: number, license: string, toolCalling: boolean, notes: string, projectorMb: number | null = null): CatalogModel {
  const file = `${name.replace(/ /g, '-')}-Q4_K_M.gguf`;
  return {
    id,
    name,
    family: id.split('-')[0] ?? id,
    paramsB,
    activeParamsB: null,
    quant: 'Q4_K_M',
    hfRepo: `unsloth/${name.replace(/ /g, '-')}-GGUF`,
    file,
    sha256: '0'.repeat(64),
    sizeBytes: Math.round(sizeGb * GB),
    license,
    toolCalling,
    contextDefault: 8192,
    minVramGb: 0,
    minRamGb: 0,
    moe: false,
    recommendedFor: [],
    notes,
    // The model's image projector (ADR 0025): the `mmproj` file of the same repository.
    projector: projectorMb === null ? null : { file: `mmproj-${name.replace(/ /g, '-')}-F16.gguf`, sha256: '1'.repeat(64), sizeBytes: Math.round(projectorMb * MB) },
  };
}

export const MOCK_CATALOG: CatalogModel[] = [
  model('qwen3.5-9b-q4km', 'Qwen3.5 9B', 9, 5.6, 'apache-2.0', true, 'Best quality that fits 8 GB of video memory.', 876),
  model('qwen3.5-4b-q4km', 'Qwen3.5 4B', 4, 2.7, 'apache-2.0', true, 'Fast; good tool calling.', 640),
  model('llama3.2-3b-q4km', 'Llama 3.2 3B', 3.2, 2.0, 'llama3.2', true, 'Small and quick.'),
  model('gemma3-12b-q4km', 'Gemma 3 12B', 12, 7.3, 'gemma', false, 'Strong writer; no tool calling.', 812),
  model('qwen3.5-32b-q4km', 'Qwen3.5 32B', 32, 19.8, 'apache-2.0', true, 'Needs a 24 GB GPU.'),
  { ...model('qwen3.6-35b-a3b-q4km', 'Qwen3.6 35B-A3B', 35, 20.6, 'apache-2.0', true, 'Best quality, text + images; needs 32 GB of RAM.', 858), activeParamsB: 3, moe: true, minRamGb: 32 },
];

/** The catalog entry at `index` (the mock catalog is fixed, so a missing one is a programming error). */
export function catalogModel(index: number): CatalogModel {
  const m = MOCK_CATALOG[index];
  if (!m) throw new Error(`no mock catalog model at ${index}`);
  return m;
}

export function mockRecommendations(installed: string[]): Recommendations {
  const assess = (m: CatalogModel): Recommendation => {
    const need = m.sizeBytes / GB + 1;
    const fit = need <= 8 * 0.88 ? 'comfortable' : need <= 8 + 0.4 * MOCK_HARDWARE.availableRamGb ? 'tight' : 'tooLarge';
    const speed = fit === 'comfortable' ? 'fast' : fit === 'tight' ? 'moderate' : 'slow';
    return { model: m, assessment: { needGb: Math.round(need * 10) / 10, fit, speed }, installed: installed.includes(m.file), context: fit === 'comfortable' ? 8192 : 4096 };
  };
  const ranked = MOCK_CATALOG.map(assess);
  return {
    hardware: '32 GB RAM, 16 cores, NVIDIA GeForce RTX 4060 Laptop GPU with 8 GB video memory',
    budget: { usableGb: 8, class: 'discreteGpu' },
    best: ranked[0] ?? null,
    fastest: ranked[1] ?? null,
    ranked,
    message: 'Qwen3.5 9B fits comfortably.',
  };
}

export function installedFrom(m: CatalogModel, active: boolean): InstalledModel {
  return {
    fileName: m.file,
    sizeBytes: m.sizeBytes,
    info: { version: 3, architecture: 'qwen3', name: m.name, quant: m.quant, contextLength: 32768, blockCount: 36, tensorCount: 398, fileSize: m.sizeBytes, paramsB: m.paramsB, knownArchitecture: true },
    problem: null,
    active,
    assessment: mockRecommendations([]).ranked.find((r) => r.model.id === m.id)?.assessment ?? null,
    catalogId: m.id,
    name: m.name,
    license: m.license,
    toolCalling: m.toolCalling,
    // Installed with its projector when the catalog has one (ADR 0025: one install downloads both).
    vision: !!m.projector,
    projector: m.projector ? { fileName: `${m.file.replace(/\.gguf$/i, '')}.mmproj.gguf`, sizeBytes: m.projector.sizeBytes, info: mockProjectorInfo(m.projector.sizeBytes), problem: null } : null,
    projectorDownloadBytes: null,
  };
}

/** What a projector's header says (the mock's are all Qwen-style CLIP encoders). */
export function mockProjectorInfo(fileSize: number): ProjectorInfo {
  return { name: 'mmproj', projectorType: 'qwen3vl_merger', projectionDim: 2560, imageSize: 768, patchSize: 16, spatialMergeSize: 2, fileSize };
}

/** The same model without its projector: text only, with "Add image support" offered when the catalog has one. */
export function textOnly(m: InstalledModel, catalog: CatalogModel | undefined): InstalledModel {
  return { ...m, vision: false, projector: null, projectorDownloadBytes: catalog?.projector?.sizeBytes ?? null };
}

let demoIds: { error: number; fatal: number; warning: number } | null = null;

/** Ids of real records of the demo workspace (same seed as the mock backend), so citations jump somewhere. */
export function demoRecordIds(): { error: number; fatal: number; warning: number } {
  if (!demoIds) {
    // Ids as the mock engine assigns them: the position in time order (the demo profile drops nothing).
    const records = [...generateDemo().records].sort((a, b) => (a.ts ?? Number.MAX_SAFE_INTEGER) - (b.ts ?? Number.MAX_SAFE_INTEGER) || a.source - b.source || a.line - b.line);
    const first = (level: string, fallback: number) => {
      const i = records.findIndex((r) => r.level === level);
      return i >= 0 ? i : fallback;
    };
    demoIds = { error: first('Error', 1), fatal: first('Fatal', 2), warning: first('Warning', 3) };
  }
  return demoIds;
}

export function mockRegexProposal(lines: string[]): RegexProposal {
  const pattern = String.raw`^(?<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?) \[(?<level>\w+)\] (?<message>.*)$`;
  const re = new RegExp(pattern.replace(/\(\?<(\w+)>/g, '('));
  const matches = lines.map((l) => re.test(l));
  return {
    kind: 'regex',
    pattern,
    source: 'model',
    groups: [
      { name: 'timestamp', kind: 'timestamp', examples: ['2026-10-14 09:00:02'] },
      { name: 'level', kind: 'word', examples: ['INF', 'ERR'] },
      { name: 'message', kind: 'text', examples: ['Order 1182 accepted'] },
    ],
    matches,
    matchCount: matches.filter(Boolean).length,
    total: lines.length,
    explanation: [
      { depth: 0, text: 'Start of the line', start: 0, end: 1 },
      { depth: 0, text: 'Group "timestamp": a date and time', start: 1, end: 60 },
      { depth: 0, text: 'Group "level" in square brackets', start: 61, end: 75 },
      { depth: 0, text: 'Group "message": the rest of the line', start: 76, end: 92 },
    ],
    rationale: 'Each line starts with a timestamp, then the level in brackets, then the message.',
  };
}

/** The parser the mock AI proposes for the demo's worker log (invented). */
const MOCK_PROPOSED_PARSER = String.raw`schema = 1
id = "worker-text"
name = "Worker text logs"
version = "1.0.0"
description = "Plain text with an ISO timestamp and a bracketed level, proposed by the local model."
category = "Custom"

[match]
files = ["worker-*.log"]

[format]
kind = "text"
regexes = ['^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?(?: [+-]\d{2}:\d{2})?) \[(?P<level>[A-Z]{3})\] (?P<message>.*)$']
`;

export function mockParserProposal(): ParserProposal {
  return {
    kind: 'parser',
    name: 'Worker text logs',
    // A parser for the library (ADR 0026); empty when the library already reads the file.
    toml: MOCK_PROPOSED_PARSER,
    importer: 'text',
    parsed: 58,
    withTimestamp: 58,
    preview: [
      { line: 1, ts: Date.UTC(2026, 9, 14, 9, 0, 2), level: 'Information', message: 'Worker started', exception: null, fields: { Worker: 'sync' }, markers: ['start'] },
      { line: 2, ts: Date.UTC(2026, 9, 14, 9, 0, 3), level: 'Warning', message: 'Queue depth 812 above 500', exception: null, fields: {}, markers: ['warning'] },
    ],
    rationale: 'The sample is plain text with an ISO timestamp and a bracketed level.',
  };
}

export function mockPipelineProposal(goal: string): PipelineProposal {
  const toml = `[[pipeline]]\nstep = "drop"\nwhen = [{ field = "@message", op = "contains", value = "health check" }]\n`;
  return {
    kind: 'pipeline',
    toml,
    profileToml: `${MOCK_PROFILE_TOML}\n${toml}`,
    steps: [{ index: 0, step: 'drop: @message contains "health check"', input: 60, output: 52 }],
    sampleTotal: 60,
    kept: 52,
    dropped: 8,
    changed: 0,
    examples: [{ before: 'GET /health check ok in 2 ms', after: '(dropped)' }],
    rationale: `Health checks are noise for "${goal}"; they are dropped before analysis.`,
  };
}

export function mockMarkersProposal(goal: string): MarkersProposal {
  const toml = `[[marker]]\nkind = "timeout"\nlabel = "Timeout"\ncolor = "#E9AE3F"\nwhen = [{ field = "@message", op = "contains", value = "timed out" }]\n`;
  return {
    kind: 'markers',
    toml,
    profileToml: `${MOCK_PROFILE_TOML}\n${toml}`,
    markers: [{ kind: 'timeout', label: 'Timeout', matches: 6 }],
    sampled: 60,
    rationale: `Timeouts are what "${goal}" is about; they become markers on the timeline.`,
  };
}

export function mockExtractorsProposal(goal: string): ExtractorsProposal {
  const toml = `[[extract]]\nname = "elapsed"\nfrom = "@message"\nregex = 'in (?P<elapsed>\\d+) ms'\n[extract.fields]\nelapsed = { type = "duration", unit = "ms" }\n`;
  return {
    kind: 'extractors',
    toml,
    profileToml: `${MOCK_PROFILE_TOML}\n${toml}`,
    extractors: [{ name: 'elapsed', fields: ['elapsed'], matches: 31, examples: [{ elapsed: 120 }, { elapsed: 2300 }] }],
    sampled: 60,
    rationale: `Request durations answer "${goal}".`,
  };
}
