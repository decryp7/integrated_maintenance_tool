import { effect, EnvironmentProviders, inject, Injector, makeEnvironmentProviders, provideEnvironmentInitializer } from '@angular/core';
import { isTauri } from '../core/platform';
import { TimeCursor } from '../analysis/time-cursor';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { IMT_RECORD_MENU_ITEMS, RecordMenuContribution } from '../context/record-actions';
import { IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { AiBackend, provideAiBackend } from './ai-backend';
import { AiChatPersistence } from './ai-chat-persistence';
import { AiLauncher } from './ai-launcher';
import { AiStatusStore } from './ai-status.store';

export const AI_CHAT_VIEW = 'ai-chat';

/** The chat, a tool window offered only while AI can be used in this run. */
export function aiViews(status: InstanceType<typeof AiStatusStore>): ImtView[] {
  return [
    {
      id: AI_CHAT_VIEW,
      title: $localize`:@@ai.provide.view.title:AI assistant`,
      category: 'AI',
      icon: 'puzzle',
      defaultPosition: 'right',
      component: () => import('./chat/chat-panel').then((m) => m.AiChatPanel),
      enabled: () => status.available(),
    },
  ];
}

/** The range to summarize: the brushed selection, else the query's range, else the whole workspace. */
export function rangeToSummarize(cursor: TimeCursor, store: InstanceType<typeof WorkspaceStore>): { from: number; to: number } | null {
  const sel = cursor.selection();
  if (sel && sel.to > sel.from) return sel;
  const q = store.query();
  const s = store.summary();
  const from = q.from ?? s?.from ?? null;
  const to = q.to ?? s?.to ?? null;
  return from !== null && to !== null && to > from ? { from, to } : null;
}

export function aiCommands(
  launcher: AiLauncher,
  status: InstanceType<typeof AiStatusStore>,
  store: InstanceType<typeof WorkspaceStore>,
  cursor: TimeCursor,
): ImtCommand[] {
  const available = () => status.available();
  const withData = () => status.available() && !!store.summary()?.total;
  const selected = (): RecordView | null => store.selected();
  return [
    { id: 'ai.chat.show', title: $localize`:@@ai.provide.cmd.chat:AI: Open the assistant chat`, category: $localize`:@@ai.provide.category:AI`, when: available, run: (ctx) => ctx.showView(AI_CHAT_VIEW) },
    { id: 'ai.setup', title: $localize`:@@ai.provide.cmd.setup:Set up AI assistant`, category: $localize`:@@ai.provide.category:AI`, when: available, run: () => launcher.openSetup() },
    { id: 'ai.models', title: $localize`:@@ai.provide.cmd.models:AI: Manage models…`, category: $localize`:@@ai.provide.category:AI`, when: available, run: () => launcher.openModels() },
    {
      id: 'ai.explain',
      title: $localize`:@@ai.provide.cmd.explain:AI: Explain the selected event`,
      category: $localize`:@@ai.provide.category:AI`,
      when: () => available() && !!selected(),
      run: () => {
        const r = selected();
        if (r) void launcher.explainEvent(r.id);
      },
    },
    {
      id: 'ai.summarize',
      title: $localize`:@@ai.provide.cmd.summarize:AI: Summarize this range`,
      category: $localize`:@@ai.provide.category:AI`,
      when: () => withData() && !!rangeToSummarize(cursor, store),
      run: () => {
        const r = rangeToSummarize(cursor, store);
        if (r) void launcher.summarize(r.from, r.to);
      },
    },
    { id: 'ai.query', title: $localize`:@@ai.provide.cmd.query:AI: Ask AI to write a query…`, category: $localize`:@@ai.provide.category:AI`, when: withData, run: () => launcher.askQuery() },
    { id: 'ai.rootCauses', title: $localize`:@@ai.provide.cmd.rootCauses:AI: Find likely root causes`, category: $localize`:@@ai.provide.category:AI`, when: withData, run: () => launcher.rootCauses() },
    {
      id: 'ai.propose.pipeline',
      title: $localize`:@@ai.provide.cmd.pipeline:AI: Propose pipeline steps…`,
      category: $localize`:@@ai.provide.category:AI`,
      when: () => available() && !!selected(),
      run: () => launcher.propose('pipeline', { recordId: selected()?.id ?? null }),
    },
    {
      id: 'ai.propose.markers',
      title: $localize`:@@ai.provide.cmd.markers:AI: Propose markers…`,
      category: $localize`:@@ai.provide.category:AI`,
      when: () => available() && !!selected(),
      run: () => launcher.propose('markers', { recordId: selected()?.id ?? null }),
    },
    {
      id: 'ai.propose.extractors',
      title: $localize`:@@ai.provide.cmd.extractors:AI: Propose extractors…`,
      category: $localize`:@@ai.provide.category:AI`,
      when: () => available() && !!selected(),
      run: () => launcher.propose('extractors', { recordId: selected()?.id ?? null }),
    },
  ];
}

/** "Explain this event" and "Suggest regex" in the grid and inspector context menu. */
export function aiRecordMenu(launcher: AiLauncher, status: InstanceType<typeof AiStatusStore>): RecordMenuContribution {
  return (record, target) => {
    if (!status.available()) return [];
    const lines = (target.records?.length ? target.records : [record]).map((r) => r.message);
    return [
      { label: $localize`:@@ai.provide.menu.explain:Explain this event (AI)…`, run: () => void launcher.explainEvent(record.id) },
      { label: $localize`:@@ai.provide.menu.regex:Suggest a regex for these lines (AI)…`, run: () => void launcher.suggestRegex(lines, { highlight: true }) },
    ];
  };
}

const FIRST_RUN_KEY = 'imt.ai.setupOffered.v1';

/**
 * First run of the desktop app: when AI can be used but no model is set up, the set-up wizard is offered
 * once (the browser build never opens it by itself, so UI tests stay deterministic).
 */
export function offerSetupOnFirstRun(status: InstanceType<typeof AiStatusStore>, launcher: AiLauncher, injector: Injector, desktop = isTauri()): void {
  if (!desktop) return;
  let offered: boolean;
  try {
    offered = localStorage.getItem(FIRST_RUN_KEY) === '1';
  } catch {
    offered = true; // no storage: never nag on every start
  }
  if (offered) return;
  const ref = effect(
    () => {
      const s = status.status();
      if (!s) return;
      ref.destroy();
      if (!s.available || s.state !== 'noModel') return;
      try {
        localStorage.setItem(FIRST_RUN_KEY, '1');
      } catch {
        /* storage unavailable: the offer may repeat, which is harmless */
      }
      void launcher.openSetup();
    },
    { injector, manualCleanup: true },
  );
}

/** One line in `app.config.ts`: the AI chat view, its commands and menu items, saved chats, the backend. */
export function provideAi(): EnvironmentProviders {
  return makeEnvironmentProviders([
    ...provideAiBackend(),
    { provide: IMT_VIEWS, multi: true, useFactory: () => aiViews(inject(AiStatusStore)) },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => aiCommands(inject(AiLauncher), inject(AiStatusStore), inject(WorkspaceStore), inject(TimeCursor)) },
    { provide: IMT_RECORD_MENU_ITEMS, multi: true, useFactory: () => aiRecordMenu(inject(AiLauncher), inject(AiStatusStore)) },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(AiChatPersistence).contribution] },
    provideEnvironmentInitializer(() => offerSetupOnFirstRun(inject(AiStatusStore), inject(AiLauncher), inject(Injector))),
  ]);
}

export { AiBackend };
