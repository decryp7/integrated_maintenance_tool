import { useCases } from './uc.mjs';

useCases('uml-use-cases-investigate', {
  title: 'Use cases — investigating', bound: 'Debrief',
  desc: 'What a support engineer does with logs and diagnostics. A developer can do everything a support engineer does.',
  rows: ['Open logs, folders and archives', 'Open only a time range', 'Filter and query', 'Analyse the timeline', 'Inspect an event in context', 'Follow live logs with alerts', 'Find anomalies, patterns and outliers', 'Compare time windows (Moment)', 'Take notes on events and images', 'Research in a Chromium browser', 'Follow long tasks (Jobs)'],
  actors: [{ name: 'Support engineer', rows: [0, 10] }, { name: 'Developer' }], general: [[1, 0]],
  sats: [{ text: 'Parse with a profile', row: 0, from: 0, kind: '«include»' }, { text: 'Use a lens or filter set', row: 2, from: 2, kind: '«extend»' }],
});
useCases('uml-use-cases-report', {
  title: 'Use cases — explaining and sharing', bound: 'Debrief',
  desc: 'Turning what was found into something someone else can read, open or act on.',
  rows: ['Declare a known issue', 'Write an incident report (HTML, PDF)', 'Share a bundle (`.debriefz`)', 'Save and open projects', 'Correct clock skew', 'Ask the local AI assistant', 'Open crash dumps and UMDH logs'],
  actors: [{ name: 'Support engineer', rows: [0, 6] }],
  sats: [{ text: 'Redact sensitive data', row: 1, from: [1, 2], kind: '«include»' }, { text: 'Set up a local model', row: 5, from: 5, kind: '«include»' }],
});
useCases('uml-use-cases-extend', {
  title: 'Use cases — teaching and extending', bound: 'Debrief',
  desc: 'Developers teach Debrief a format and shape what it shows; plugin authors extend it in Rust.',
  rows: ['Teach a format (parsers and profiles)', 'Clean and enrich with a pipeline', 'Define lenses and filter sets', 'Build dashboards and charts', 'Profile apps and analyse captures', 'Compare configurations', 'Write a plugin (Rust SDK)'],
  actors: [{ name: 'Developer', rows: [0, 5] }, { name: 'Plugin author', rows: [6, 6] }],
  sats: [{ text: 'Preview the extraction', row: 0, from: 0, kind: '«include»' }, { text: 'Test and package a plugin', row: 6, from: 6, kind: '«include»' }],
});
useCases('uml-use-cases-operate', {
  title: 'Use cases — operating and automating', bound: 'Debrief',
  desc: 'Collecting from servers, watching folders and scripting the engine without the window.',
  rows: ['Schedule collection (`debrief-agent`)', 'Collect a diagnostics bundle', 'Monitor processes and counters', 'Control Performance Monitor', 'Follow the Windows Event Log', 'Watch folders for new files', 'Install and manage plugins', 'Install updates', 'Run `debrief` in scripts and CI'],
  actors: [{ name: 'Administrator', rows: [0, 7] }, { name: 'Script or CI', rows: [8, 8] }],
  sats: [{ text: 'Redact sensitive data', row: 1, from: 1, kind: '«include»' }, { text: 'Announce a known issue', row: 5, from: 5, kind: '«extend»' }, { text: 'Install a signed plugin', row: 6, from: 6, kind: '«extend»' }],
});
