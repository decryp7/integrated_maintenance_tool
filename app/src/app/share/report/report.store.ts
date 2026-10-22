import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { Report, ReportBlock, ReportSection } from '../../core/api.types';
import { UndoStack } from '../../undo/undo-stack';
import { recordSnapshot } from '../../undo/snapshot-undo';
import { addBlock, moveBlock, moveSection, newReport, newSection, NewBlock, parseReports, removeBlock, updateBlock, withId } from './report-model';

interface ReportState {
  reports: Report[];
  activeId: string | null;
}

/**
 * The project's incident reports and the one being edited. Saved with the project (`save_project`
 * `reports`) and the autosaved session; the engine validates and exports them.
 */
export const ReportStore = signalStore(
  { providedIn: 'root' },
  withState<ReportState>({ reports: [], activeId: null }),
  withComputed((store) => ({
    active: computed(() => store.reports().find((r) => r.id === store.activeId()) ?? store.reports()[0] ?? null),
    /** Serialized reports: compared to tell whether the project changed. */
    json: computed(() => JSON.stringify(store.reports())),
  })),
  withMethods((store, undo = inject(UndoStack)) => {
    // Report edits are undoable as snapshots of the reports (which report is active is put back too).
    const snapshot = { read: (): ReportState => ({ reports: store.reports(), activeId: store.activeId() }), write: (v: ReportState) => patchState(store, v) };
    const edit = <T>(label: string, mutate: () => T, mergeKey?: string): T => {
      let out!: T;
      recordSnapshot(undo, snapshot, label, () => (out = mutate()), { mergeKey });
      return out;
    };
    const current = () => store.reports().find((r) => r.id === store.activeId()) ?? store.reports()[0] ?? null;
    const replaceReport = (next: Report) =>
      patchState(store, (s) => ({ reports: s.reports.map((r) => (r.id === next.id ? next : r)), activeId: next.id }));
    /** The report to add to: the active one, or a new one. */
    const target = (): Report => {
      const r = current();
      if (r) return r;
      const created = newReport();
      patchState(store, (s) => ({ reports: [...s.reports, created], activeId: created.id }));
      return created;
    };
    return {
      /** Replaces every report (project opened, new project); untrusted input is checked. */
      replace(value: unknown) {
        const reports = parseReports(value);
        patchState(store, { reports, activeId: reports[0]?.id ?? null });
      },
      create(title?: string): Report {
        const r = newReport(title);
        edit($localize`:@@share.reportStore.undo.newReport:new report`, () => patchState(store, (s) => ({ reports: [...s.reports, r], activeId: r.id })));
        return r;
      },
      /** Adds a finished report (the incident summary the engine built) and makes it the active one. */
      adopt(report: Report): Report {
        const [r] = parseReports([report]);
        if (!r) return report;
        edit($localize`:@@share.reportStore.undo.reportAdoption:incident summary`, () => patchState(store, (s) => ({ reports: [...s.reports, r], activeId: r.id })));
        return r;
      },
      /** Puts a section first in the report `id` (the incident summary's AI narrative). */
      prependSection(id: string, section: ReportSection) {
        const r = store.reports().find((x) => x.id === id);
        if (r) edit($localize`:@@share.reportStore.undo.reportNarrative:report narrative`, () => replaceReport({ ...r, sections: [section, ...r.sections], updatedAt: Date.now() }));
      },
      setActive: (id: string) => patchState(store, { activeId: id }),
      remove(id: string) {
        edit($localize`:@@share.reportStore.undo.reportDeletion:report deletion`, () =>
          patchState(store, (s) => {
            const reports = s.reports.filter((r) => r.id !== id);
            return { reports, activeId: s.activeId === id ? (reports[0]?.id ?? null) : s.activeId };
          }),
        );
      },
      update(id: string, patch: Partial<Pick<Report, 'title' | 'summary'>>) {
        const r = store.reports().find((x) => x.id === id);
        if (r) edit($localize`:@@share.reportStore.undo.reportEdit:report edit`, () => replaceReport({ ...r, ...patch, updatedAt: Date.now() }), `report:${id}:${Object.keys(patch).join(',')}`);
      },
      /** Adds a block to the active report (creating one when there is none); returns the block. */
      add(block: NewBlock, sectionId?: string | null): ReportBlock {
        const b = withId(block);
        edit($localize`:@@share.reportStore.undo.reportBlockAddition:report block addition`, () => replaceReport(addBlock(target(), b, sectionId)));
        return b;
      },
      updateBlock(id: string, patch: Partial<ReportBlock>) {
        const r = current();
        if (r) edit($localize`:@@share.reportStore.undo.reportBlockEdit:report block edit`, () => replaceReport(updateBlock(r, id, patch)), `block:${id}:${Object.keys(patch).join(',')}`);
      },
      removeBlock(id: string) {
        const r = current();
        if (r) edit($localize`:@@share.reportStore.undo.reportBlockRemoval:report block removal`, () => replaceReport(removeBlock(r, id)));
      },
      moveBlock(id: string, delta: -1 | 1) {
        const r = current();
        if (r) edit($localize`:@@share.reportStore.undo.reportBlockMove:report block move`, () => replaceReport(moveBlock(r, id, delta)));
      },
      addSection(title?: string): ReportSection {
        const s = newSection(title);
        edit($localize`:@@share.reportStore.undo.reportSectionAddition:report section addition`, () => {
          const r = target();
          replaceReport({ ...r, sections: [...r.sections, s], updatedAt: Date.now() });
        });
        return s;
      },
      renameSection(id: string, title: string) {
        const r = current();
        if (r) edit($localize`:@@share.reportStore.undo.reportSectionRename:report section rename`, () => replaceReport({ ...r, sections: r.sections.map((s) => (s.id === id ? { ...s, title } : s)), updatedAt: Date.now() }), `section:${id}`);
      },
      removeSection(id: string) {
        const r = current();
        if (r) edit($localize`:@@share.reportStore.undo.reportSectionRemoval:report section removal`, () => replaceReport({ ...r, sections: r.sections.filter((s) => s.id !== id), updatedAt: Date.now() }));
      },
      moveSection(id: string, delta: -1 | 1) {
        const r = current();
        if (r) edit($localize`:@@share.reportStore.undo.reportSectionMove:report section move`, () => replaceReport(moveSection(r, id, delta)));
      },
    };
  }),
);

export type ReportStoreType = InstanceType<typeof ReportStore>;
