import { InjectionToken } from '@angular/core';
import { DialogDef } from './dialog-def';
/**
 * Every dialog of Debrief (ADR 0050): id, modality and the lazy import of its content. A dialog component
 * declares the same id as `static readonly dialogId`; a test keeps the two in step. The ADR lists why each is modal
 * or modeless. Dialogs that stream or load results (an answer, a proposal, a summary) name a height here so they never
 * open fitted to their short loading state; small decision dialogs omit it and are fitted to their content. Adding a dialog = a component with a `dialogId` and one line here.
 */
export const DIALOG_DEFS: readonly DialogDef[] = [
  { id: 'settings', modality: 'modeless', width: 960, height: 680, features: ['summary'], load: () => import('../settings/settings-screen').then((m) => m.SettingsScreen) },
  { id: 'parser-editor', modality: 'modeless', width: 820, features: ['summary'], load: () => import('../settings/parsers/parser-editor').then((m) => m.ParserEditor) },
  { id: 'parser-import-confirm', modality: 'modal', width: 520, load: () => import('../settings/parsers/parser-import-confirm').then((m) => m.ParserImportConfirm) },
  { id: 'format-wizard', modality: 'modeless', width: 760, height: 640, features: ['summary'], load: () => import('../format/format-wizard').then((m) => m.FormatWizard) },
  { id: 'whats-new', modality: 'modeless', width: 560, height: 560, features: ['tools'], load: () => import('../whats-new/whats-new').then((m) => m.WhatsNewDialog) },
  { id: 'about', modality: 'modeless', width: 720, height: 640, features: ['tools'], load: () => import('../about/about-dialog').then((m) => m.AboutDialog) },
  { id: 'updates', modality: 'modeless', width: 560, features: ['updates'], load: () => import('../updates/updates-dialog').then((m) => m.UpdatesDialog) },
  { id: 'cheat-sheet', modality: 'modeless', width: 640, load: () => import('../help/cheat-sheet').then((m) => m.CheatSheet) },
  { id: 'package-review', modality: 'modal', width: 600, features: ['tools'], load: () => import('../plugins-panel/package-review-dialog').then((m) => m.PackageReviewDialog) },
  { id: 'plugin-permissions', modality: 'modal', width: 560, features: ['tools'], load: () => import('../plugins-panel/plugin-permissions-dialog').then((m) => m.PluginPermissionsDialog) },
  { id: 'clock', modality: 'modeless', width: 560, instanced: true, features: ['skew', 'summary'], load: () => import('../skew/clock-dialog').then((m) => m.ClockDialog) },
  { id: 'collector', modality: 'modeless', width: 760, features: ['tools', 'summary'], load: () => import('../tools/collector/collector-dialog').then((m) => m.CollectorDialog) },
  { id: 'drop', modality: 'modal', width: 640, features: ['summary'], load: () => import('../context/drop-dialog').then((m) => m.DropDialog) },
  { id: 'mark', modality: 'modal', width: 760, load: () => import('../context/mark-dialog').then((m) => m.MarkDialog) },
  { id: 'infer', modality: 'modal', width: 760, load: () => import('../markers/regex/infer-dialog').then((m) => m.InferDialog) },
  { id: 'mask', modality: 'modal', width: 640, load: () => import('../context/mask-dialog').then((m) => m.MaskDialog) },
  { id: 'extract', modality: 'modal', width: 640, load: () => import('../context/extract-dialog').then((m) => m.ExtractDialog) },
  { id: 'marker-override', modality: 'modal', width: 480, load: () => import('../markers/marker-override-dialog').then((m) => m.MarkerOverrideDialog) },
  { id: 'encoding', modality: 'modal', width: 520, load: () => import('../sources/encoding/encoding-dialog').then((m) => m.EncodingDialog) },
  { id: 'open-live', modality: 'modal', width: 480, load: () => import('../live/open-live-dialog').then((m) => m.OpenLiveDialog) },
  { id: 'incident-summary', modality: 'modeless', width: 720, height: 560, load: () => import('../investigate/incident/incident-summary-dialog').then((m) => m.IncidentSummaryDialog) },
  { id: 'unsaved-changes', modality: 'modal', width: 440, load: () => import('../projects/unsaved-changes').then((m) => m.UnsavedChangesDialog) },
  { id: 'recent-projects', modality: 'modal', width: 520, features: ['tools'], load: () => import('../projects/recent-dialog').then((m) => m.RecentProjectsDialog) },
  { id: 'note-editor', modality: 'modeless', width: 620, load: () => import('../notes/note-editor').then((m) => m.NoteEditor) },
  { id: 'search-web', modality: 'modeless', width: 560, load: () => import('../browser/search-web-dialog').then((m) => m.SearchWebDialog) },
  { id: 'research', modality: 'modeless', width: 720, height: 560, load: () => import('../browser/research-dialog').then((m) => m.ResearchDialog) },
  { id: 'ai-setup', modality: 'modal', width: 760, height: 640, features: ['ai'], load: () => import('../ai/setup/setup-wizard').then((m) => m.AiSetupWizard) },
  { id: 'ai-models', modality: 'modeless', width: 820, height: 640, features: ['ai'], load: () => import('../ai/models/model-manager').then((m) => m.AiModelManager) },
  { id: 'ai-query', modality: 'modeless', width: 640, height: 560, features: ['ai'], load: () => import('../ai/dialogs/query-dialog').then((m) => m.AiQueryDialog) },
  { id: 'ai-narrative', modality: 'modeless', width: 680, height: 560, features: ['ai', 'analysis'], instanced: true, load: () => import('../ai/dialogs/narrative-dialog').then((m) => m.AiNarrativeDialog) },
  { id: 'ai-proposal-result', modality: 'modeless', width: 760, height: 560, features: ['ai', 'analysis'], instanced: true, load: () => import('../ai/dialogs/proposal-result-dialog').then((m) => m.AiProposalResultDialog) },
  { id: 'ai-regex', modality: 'modeless', width: 640, height: 480, features: ['ai'], instanced: true, load: () => import('../ai/dialogs/regex-dialog').then((m) => m.AiRegexDialog) },
  { id: 'ai-propose', modality: 'modeless', width: 760, height: 520, features: ['ai'], instanced: true, load: () => import('../ai/dialogs/propose-dialog').then((m) => m.AiProposeDialog) },
  { id: 'folder-setup', modality: 'modeless', width: 720, features: ['ai'], load: () => import('../ai/folder-setup/folder-setup-wizard').then((m) => m.FolderSetupWizard) },
  { id: 'knowledge-editor', modality: 'modeless', width: 720, load: () => import('../ai/knowledge/knowledge-editor').then((m) => m.KnowledgeEditor) },
  { id: 'profile-native', modality: 'modeless', width: 720, features: ['tools', 'profiling', 'native'], load: () => import('../profiling/native/native-dialog').then((m) => m.NativeDialog) },
  { id: 'profile-etl', modality: 'modeless', width: 600, instanced: true, features: ['tools', 'profiling', 'native'], load: () => import('../profiling/native/etl-dialog').then((m) => m.EtlDialog) },
  { id: 'profile-dotnet', modality: 'modeless', width: 720, features: ['tools', 'profiling', 'dotnet'], load: () => import('../profiling/dotnet/dotnet-dialog').then((m) => m.DotnetDialog) },
  { id: 'report-problem', modality: 'modeless', width: 680, features: ['diagnostics'], load: () => import('../diagnostics/report-problem-dialog').then((m) => m.ReportProblemDialog) },
  { id: 'alert-rules', modality: 'modeless', width: 760, load: () => import('../live/alert-rules-editor').then((m) => m.AlertRulesEditor) },
  { id: 'paste', modality: 'modal', width: 640, load: () => import('../sources/paste/paste-dialog').then((m) => m.PasteDialog) },
  { id: 'open-ask', modality: 'modal', width: 460, load: () => import('../windowing/open-ask-dialog').then((m) => m.OpenAskDialog) },
  { id: 'close-prompt', modality: 'modal', requiredChoice: true, width: 460, load: () => import('../windowing/close-prompt').then((m) => m.ClosePrompt) },
  { id: 'export', modality: 'modeless', width: 480, features: ['share', 'analysis', 'summary'], load: () => import('../export/export-dialog').then((m) => m.ExportDialog) },
  { id: 'import-range', modality: 'modal', width: 480, load: () => import('../import/import-range-dialog').then((m) => m.ImportRangeDialog) },
  { id: 'save-layout', modality: 'modal', width: 420, load: () => import('../dock/layout-dialogs').then((m) => m.SaveLayoutDialog) },
  { id: 'manage-layouts', modality: 'modeless', width: 520, load: () => import('../dock/layout-dialogs').then((m) => m.ManageLayoutsDialog) },
  { id: 'add-tile', modality: 'modal', width: 520, features: ['summary'], load: () => import('../share/dashboard/add-tile-dialog').then((m) => m.AddTileDialog) },
  { id: 'redaction', modality: 'modal', width: 640, features: ['share', 'analysis'], load: () => import('../share/redaction/redaction-dialog').then((m) => m.RedactionDialog) },
  { id: 'report-export', modality: 'modeless', width: 520, features: ['share', 'analysis'], load: () => import('../share/report/report-export-dialog').then((m) => m.ReportExportDialog) },
  { id: 'redacted-copy', modality: 'modeless', width: 760, height: 680, minWidth: 560, minHeight: 420, features: ['share', 'analysis', 'tools'], load: () => import('../share/redacted-copy/redacted-copy-dialog').then((m) => m.RedactedCopyDialog) },
];

const BY_ID = new Map(DIALOG_DEFS.map((d) => [d.id, d]));

/** The definition of dialog `id`. */
export function dialogDef(id: string): DialogDef | undefined {
  return BY_ID.get(id);
}

/** Where the dialog shell finds definitions (tests provide their own). */
export const DIALOG_LOOKUP = new InjectionToken<(id: string) => DialogDef | undefined>('DIALOG_LOOKUP', { providedIn: 'root', factory: () => dialogDef });
