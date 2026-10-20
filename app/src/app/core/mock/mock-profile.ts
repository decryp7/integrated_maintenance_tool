/* Demo profile used by the mock backend: the built-in default plus an example extractor and charts. */

export const MOCK_PROFILE_TOML = String.raw`# IMT parsing profile.
#
# A profile is a pipeline: which files to read, how to parse them into events,
# what identifies an instance, which values to extract, which events are
# important (markers) and which charts to draw. Everything an extractor
# produces becomes a field on the event, usable like any Serilog property.
#
# Field names in conditions/split_by/key may be any property, an extracted
# field, or a pseudo-field: @level @message @exception @template @source @line.

name = "OrderService (Serilog) — demo"

[files]
# Globs matched against the path relative to the opened folder. Extensions do
# not matter: the format is detected from the file content.
include = ["**/*"]
exclude = ["**/*-filtered.*", "**/*.imtprofile.toml", "**/*.gz", "**/*.zip", "**/*.exe", "**/*.dll", "**/.*"]

[parse]
# auto | json | text | csv
format = "auto"
# Serilog output templates (as written in appsettings.json), tried in order for
# plain-text files. Named holes other than Timestamp/Level/Message/NewLine/
# Exception/Properties become fields.
output_templates = [
  "{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}",
  "[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}",
]
# Raw regexes with named groups (timestamp, level, message; others become
# fields), tried after the templates.
regexes = [
  # log4net / NLog: %date [%thread] %-5level %logger [%ndc] - %message
  '^(?P<timestamp>\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d{1,9})?(?: ?(?:Z|[+-]\d{2}:?\d{2}))?)\s+\[(?P<Thread>[^\]]+)\]\s+(?P<level>[A-Za-z]+)\s+(?P<Logger>\S+)\s+(?:\[?(?P<Ndc>[^\]\s]*)\]?\s+)?-\s(?P<message>.*)$',
  # Generic: timestamp, optional level, message
  '^(?P<timestamp>\d{4}[-/.]\d{2}[-/.]\d{2}[ T,]\d{2}:\d{2}:\d{2}(?:[.,]\d{1,9})?(?: ?(?:Z|[+-]\d{2}:?\d{2}))?)\s+(?:\[?(?P<level>VRB|DBG|INF|WRN|ERR|FTL|TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL|Verbose|Debug|Information|Warning|Error|Fatal)\]?\s*[:|-]?\s+)?(?P<message>.*)$',
]
# Extra timestamp formats, .NET style ("dd/MM/yyyy HH:mm:ss.fff") or strftime
# ("%d/%m/%Y %H:%M:%S%.f"). Common ISO/RFC/Serilog/epoch formats are always
# recognised.
timestamp_formats = []
# Timezone for timestamps without an offset: "UTC", "local" or "+08:00".
assume_timezone = "UTC"
# For ambiguous dates like 03/04/2026: "auto" (decided per file), "true", "false".
day_first = "auto"

[parse.json]
# JSON lines (one object per line) or a whole-file JSON array. Keys are tried in
# order; nested keys use dots ("log.level"). Covers Serilog CLEF (@t/@mt/@l/@x),
# Serilog JsonFormatter, NLog/ECS-style and most JSON loggers.
timestamp = ["@t", "Timestamp", "timestamp", "@timestamp", "time", "ts", "date", "datetime"]
level = ["@l", "Level", "level", "log.level", "severity", "lvl", "loglevel"]
message = ["@m", "RenderedMessage", "Message", "message", "msg", "text"]
template = ["@mt", "MessageTemplate"]
exception = ["@x", "Exception", "exception", "error.stack_trace", "stack_trace", "error"]
# Objects whose members are lifted to top-level fields; other nested objects
# are flattened with dotted names.
properties = ["Properties", "properties", "fields", "context"]
# CLEF omits @l for Information.
default_level = "Information"

[parse.csv]
# "auto" sniffs , ; tab and |
delimiter = "auto"
has_header = true
# Column names, or "auto" to guess from the header.
timestamp = "auto"
level = "auto"
message = "auto"

[instance]
# Fields forming the instance key. Empty = the candidates present on at least
# half of the events, falling back to the source file.
key = []
candidates = ["Application", "ApplicationName", "MachineName", "EnvironmentName", "ProcessId", "InstanceId"]

# [[extract]]
# name = "process-stats"
# from = "@message"                  # any field or pseudo-field
# regex = 'CPU (?P<cpu>[\d.]+)% .* WorkingSet (?P<ws>[\d.]+) ?(?P<ws_unit>KB|MB|GB)'
# when = [{ field = "SourceContext", op = "eq", value = "Diagnostics.ProcessMonitor" }]
# [extract.fields]
# cpu = { type = "number", unit = "%" }
# ws  = { type = "bytes", unit_from = "ws_unit", to = "MB" }
#
# Field types: string, number, integer, bool, bytes (unit / unit_from, to),
# duration (ms, s, m, h or TimeSpan "00:00:01.234"; to = ms|s).

# Markers: first matching rule (in this order) is the event's primary marker.
# Condition ops: eq ne contains notContains matches exists missing gt lt in
[[marker]]
kind = "fatal"
label = "Fatal"
color = "#FF4FA3"
when = [{ field = "@level", op = "eq", value = "Fatal" }]

[[marker]]
kind = "start"
label = "Start"
lifecycle = true
color = "#3FBF7F"
when = [{ field = "@message", op = "matches", value = '(?i)(\b(application|app|host|service|server|worker)\b.{0,40}\b(start(ed|ing)?|launch(ed|ing))\b|^start(ing|ed)\b|now listening on)' }]

[[marker]]
kind = "stop"
label = "Stop"
lifecycle = true
color = "#A7B0C2"
when = [{ field = "@message", op = "matches", value = '(?i)(\b(application|app|host|service|server|worker)\b.{0,40}\b(stop(ped|ping)?|shut(ting)? ?down|terminat(ed|ing)|exit(ed|ing))\b|^shutting down|^(stopping|stopped)\b)' }]

[[marker]]
kind = "exception"
label = "Exception"
color = "#FF7A59"
when = [{ field = "@exception", op = "exists" }]

[[marker]]
kind = "error"
label = "Error"
color = "#F2545B"
when = [{ field = "@level", op = "eq", value = "Error" }]

[[marker]]
kind = "warning"
label = "Warning"
color = "#E9AE3F"
when = [{ field = "@level", op = "eq", value = "Warning" }]

[[chart]]
id = "events-by-level"
title = "Events per interval by level"
y = "count()"
bucket = "auto"
split_by = ["@level"]
type = "stacked"
unit = "events"

[[extract]]
name = "process-stats"
from = "@message"
regex = 'CPU (?P<cpu>[\d.]+)% · WorkingSet (?P<ws>[\d.]+) ?(?P<ws_unit>KB|MB|GB) · Threads (?P<threads>\d+)'
when = [{ field = "SourceContext", op = "eq", value = "Diagnostics.ProcessMonitor" }]
[extract.fields]
cpu = { type = "number", unit = "%" }
ws = { type = "bytes", unit_from = "ws_unit", to = "MB" }
threads = { type = "integer" }

[[chart]]
id = "cpu"
title = "CPU %"
y = "avg(cpu)"
bucket = "1m"
split_by = ["MachineName", "ProcessId"]
type = "line"
unit = "%"

[[chart]]
id = "working-set"
title = "Working set"
y = "avg(ws)"
bucket = "1m"
split_by = ["MachineName", "ProcessId"]
type = "line"
unit = "MB"

# Redaction for the redacted view, exports, reports and bundles.
[redact]
detectors = ["bearer-token", "jwt", "api-key", "connection-string", "password", "user-path", "email", "credit-card", "ipv4", "ipv6", "guid"]
mode = "mask"
fields = ["@message", "@exception", "CorrelationId", "MessageId", "Address"]
on_export = false

[[dashboard]]
id = "orderservice-health"
title = "OrderService health"
columns = 12

[[dashboard.tile]]
id = "errors"
title = "Errors"
type = "stat"
query = 'level >= error | stats count() as errors'
x = 0
y = 0
w = 3
h = 2

[[dashboard.tile]]
id = "warnings"
title = "Warnings"
type = "stat"
query = 'level = warning | stats count() as warnings'
x = 3
y = 0
w = 3
h = 2

[[dashboard.tile]]
id = "p95"
title = "p95 request time"
type = "stat"
query = '| stats p95(Elapsed) as p95'
unit = "ms"
x = 6
y = 0
w = 3
h = 2

[[dashboard.tile]]
id = "noisy"
title = "Noisiest sources"
type = "top"
field = "SourceContext"
query = 'level >= warning'
limit = 6
x = 9
y = 0
w = 3
h = 4

[[dashboard.tile]]
id = "levels"
title = "Events by level"
type = "chart"
chart = "events-by-level"
x = 0
y = 2
w = 9
h = 4

[[dashboard.tile]]
id = "cpu"
title = "CPU %"
type = "chart"
chart = "cpu"
x = 0
y = 6
w = 6
h = 4

[[dashboard.tile]]
id = "slow"
title = "Slowest endpoints"
type = "table"
query = '| stats count() as requests, p95(Elapsed) as p95 by RequestPath | sort p95 desc | head 8'
x = 6
y = 6
w = 6
h = 4

[[dashboard.tile]]
id = "alerts"
title = "Alerts"
type = "alerts"
limit = 8
x = 9
y = 4
w = 3
h = 2
`;
