import { DEMO_PARSER_IMPORT } from '../../settings/parsers/parser-files';

/**
 * The browser demo's parser library: simplified stand-ins for the engine's built-in parsers
 * (crates/debrief-core/parsers, which are the reference) and a few parser files the demo "picks" from disk.
 * Every format carries a regex or JSON/CSV hints `mockPreviewParse` understands. Invented data only.
 */

interface BuiltinSpec {
  id: string;
  name: string;
  category: string;
  description: string;
  kind: string;
  files?: string[];
  priority?: number;
  regex?: string;
  /** More `[format]` keys. */
  format?: string;
  /** Sub-tables after `[format]` (`[format.json]`, `[format.csv]`). */
  extra?: string;
  version?: string;
}

/** A TOML string literal that keeps backslashes (multi-line literal strings never escape). */
const lit = (s: string): string => `'''${s}'''`;

function builtinToml(p: BuiltinSpec): string {
  const lines = ['schema = 1', `id = "${p.id}"`, `name = ${JSON.stringify(p.name)}`, `version = "${p.version ?? '1.0.0'}"`, `description = ${JSON.stringify(p.description)}`, `category = ${JSON.stringify(p.category)}`, '', '[match]'];
  if (p.files) lines.push(`files = [${p.files.map((f) => JSON.stringify(f)).join(', ')}]`);
  lines.push(`priority = ${p.priority ?? 0}`, '', '[format]', `kind = "${p.kind}"`);
  if (p.regex) lines.push(`regexes = [${lit(p.regex)}]`);
  if (p.format) lines.push(p.format);
  if (p.extra) lines.push(p.extra);
  return lines.join('\n') + '\n';
}

const ISO = String.raw`\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:\s?(?:Z|[+-]\d{2}:?\d{2}))?`;

const LOG4NET_CSV: BuiltinSpec = {
  id: 'log4net-csv',
  name: 'log4net CSV (no header)',
  category: '.NET',
  description: 'log4net CSV layouts without a header row: date, time with offset, level, logger and message.',
  kind: 'csv',
  files: ['*.csv', '*.log'],
  priority: -5,
  extra: '\n[format.csv]\ndelimiter = ","\nhas_header = false',
};

const SPECS: BuiltinSpec[] = [
  { id: 'generic', name: 'Generic', category: 'Generic', description: 'The formats Debrief understands out of the box: JSON lines, CSV with a header and timestamped text. The fallback for any file.', kind: 'auto', priority: -100, regex: String.raw`^(?P<timestamp>${ISO})\s+(?:\[?(?P<level>[A-Za-z]+)\]?\s+)?(?P<message>.*)$` },
  { id: 'serilog-clef', name: 'Serilog CLEF', category: '.NET', description: 'Compact JSON written by Serilog.Formatting.Compact: @t, @l, @mt and @x.', kind: 'json', files: ['*.clef', '*.jsonl'], extra: '\n[format.json]\ntimestamp = ["@t"]\nlevel = ["@l"]\nmessage = ["@m", "@mt"]\nexception = ["@x"]' },
  { id: 'serilog-json', name: 'Serilog JSON', category: '.NET', description: 'JSON written by Serilog.Formatting.Json.JsonFormatter: Timestamp, Level, MessageTemplate and RenderedMessage.', kind: 'json', files: ['*.json'], priority: -1, extra: '\n[format.json]\ntimestamp = ["Timestamp"]\nlevel = ["Level"]\nmessage = ["RenderedMessage", "MessageTemplate"]\nexception = ["Exception"]' },
  { id: 'serilog-text', name: 'Serilog text', category: '.NET', description: 'Plain text from the Serilog console or file sink with the default output template.', kind: 'text', files: ['*.log', '*.txt'], regex: String.raw`^(?P<timestamp>${ISO})\s+\[(?P<level>[A-Z]{3})\]\s+(?P<message>.*)$` },
  { id: 'log4net-xml', name: 'log4net XML', category: '.NET', description: "XML events written by log4net's XmlLayoutSchemaLog4j.", kind: 'text', files: ['*.xml'], regex: String.raw`timestamp="(?P<timestamp>[^"]+)" level="(?P<level>\w+)".*?<log4net:message>(?P<message>.*?)</log4net:message>` },
  { id: 'log4net-pattern', name: 'log4net pattern layout', category: '.NET', description: "Text from log4net's common PatternLayout (%date [%thread] %-5level %logger - %message).", kind: 'text', files: ['*.log'], priority: -1, regex: String.raw`^(?P<timestamp>${ISO})\s+\[(?P<thread>[^\]]+)\]\s+(?P<level>[A-Z]+)\s+(?P<logger>\S+)\s+-\s+(?P<message>.*)$` },
  LOG4NET_CSV,
  { id: 'nlog', name: 'NLog default layout', category: '.NET', description: "Text from NLog's default layout (${longdate}|${level}|${logger}|${message}).", kind: 'text', files: ['*.log'], priority: -2, regex: String.raw`^(?P<timestamp>${ISO})\|(?P<level>[A-Z]+)\|(?P<logger>[^|]*)\|(?P<message>.*)$` },
  { id: 'mel-console', name: 'Microsoft.Extensions.Logging console', category: '.NET', description: 'Output of the simple console formatter with timestamps (info: Category[EventId] message).', kind: 'text', files: ['*.log', '*.txt'], priority: -3, regex: String.raw`^(?P<timestamp>${ISO})\s+(?P<level>trce|dbug|info|warn|fail|crit):\s+(?P<message>.*)$` },
  { id: 'iis-w3c', name: 'IIS W3C extended', category: 'Web servers', description: 'IIS access logs in the W3C extended format.', kind: 'text', files: ['u_ex*.log'], regex: String.raw`^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) (?P<message>.*)$` },
  { id: 'access-combined', name: 'Apache / Nginx access (combined)', category: 'Web servers', description: 'Access logs in the combined format used by Apache httpd and Nginx.', kind: 'text', files: ['*access*.log'], regex: String.raw`^(?P<client>\S+) \S+ \S+ \[(?P<timestamp>[^\]]+)\] "(?P<message>[^"]*)" (?P<status>\d{3}) (?P<bytes>\S+)` },
  { id: 'apache-error', name: 'Apache error log', category: 'Web servers', description: 'Apache httpd error logs (2.2 and 2.4 layouts).', kind: 'text', files: ['*error*.log'], regex: String.raw`^\[(?P<timestamp>[^\]]+)\]\s+\[(?:(?P<module>[^:\]]+):)?(?P<level>\w+)\]\s+(?P<message>.*)$` },
  { id: 'syslog', name: 'Syslog', category: 'System', description: 'Syslog messages (RFC 5424, and RFC 3164 lines with a year); the priority gives the level.', kind: 'text', files: ['*syslog*', '*.log'], priority: -4, regex: String.raw`^<\d+>\d?\s*(?P<timestamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))\s+(?P<host>\S+)\s+(?P<app>\S+)\s+(?P<pid>\S+)\s+\S+\s+(?:-\s+)?(?P<message>.*)$` },
  { id: 'windows-event', name: 'Windows Event Log (EVTX)', category: 'System', description: 'Windows event logs (.evtx): provider, event id, computer and the rendered message.', kind: 'evtx', files: ['*.evtx'] },
  { id: 'android-logcat', name: 'Android logcat', category: 'Mobile', description: 'adb logcat in the threadtime format.', kind: 'text', files: ['*logcat*'], regex: String.raw`^(?P<timestamp>\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3})\s+(?P<pid>\d+)\s+(?P<tid>\d+)\s+(?P<level>[VDIWEF])\s+(?P<tag>[^:]*?)\s*:\s(?P<message>.*)$` },
  { id: 'hadoop-spark', name: 'Hadoop / Spark (log4j)', category: 'Big data', description: 'Spark and Hadoop log4j output (yy/MM/dd HH:mm:ss LEVEL Class: message).', kind: 'text', files: ['*spark*', '*hadoop*'], regex: String.raw`^(?P<timestamp>\d{2}/\d{2}/\d{2} \d{2}:\d{2}:\d{2})\s+(?P<level>[A-Z]+)\s+(?P<logger>[^:]+):\s(?P<message>.*)$` },
  { id: 'csv', name: 'CSV with a header', category: 'Generic', description: 'Comma-separated files with a header row: timestamp, level and message columns are recognised by name.', kind: 'csv', files: ['*.csv'], extra: '\n[format.csv]\ndelimiter = "auto"\nhas_header = true' },
  { id: 'plain-timestamped', name: 'Plain timestamped text', category: 'Generic', description: 'Any text log whose lines start with a date and time, optionally followed by a level.', kind: 'text', priority: -50, regex: String.raw`^(?P<timestamp>${ISO})\s+(?:\[?(?P<level>[A-Za-z]+)\]?\s+)?(?P<message>.*)$` },];

/** The built-in parsers the mock library starts with (id → TOML). */
export const MOCK_BUILTIN_PARSERS: ReadonlyMap<string, string> = new Map(SPECS.map((s) => [s.id, builtinToml(s)]));

/** Folder of the parser files on the demo "disk". */
export const MOCK_PARSER_DIR = '/demo/parsers';
/** The file "Import .toml…" picks in the browser build: an invented parser for the demo's CSV export. */
export const MOCK_IMPORT_PARSER_PATH = DEMO_PARSER_IMPORT;

/**
 * Parser files on the demo "disk": a new CSV parser for the demo's inventory export, and a newer version of the
 * built-in log4net CSV parser (to try Update).
 */
export const MOCK_PARSER_FILES: ReadonlyMap<string, string> = new Map([
  [
    MOCK_IMPORT_PARSER_PATH,
    `schema = 1
id = "inventory-csv"
name = "Inventory sync CSV"
version = "1.0.0"
description = "CSV written by the inventory sync job: date, time with offset, level, component and message, no header."
category = "Custom"

[match]
files = ["inventory-*.csv"]
sniff = '^\\d{4}/\\d{2}/\\d{2},'
priority = 10

[format]
kind = "csv"
identity = ["Component"]

[format.csv]
delimiter = ","
has_header = false
`,
  ],
  [
    `${MOCK_PARSER_DIR}/log4net-csv.toml`,
    builtinToml({ ...LOG4NET_CSV, version: '1.1.0', format: 'timestamp_formats = ["%Y/%m/%d,%H:%M:%S%.f%:z"]' }),
  ],
]);

/** A user parser file the mock library lists as broken (it does not parse). */
export const MOCK_BROKEN_PARSER = { file: 'legacy-gateway.toml', error: { message: 'missing field `id`', line: 1, column: 1 } };
