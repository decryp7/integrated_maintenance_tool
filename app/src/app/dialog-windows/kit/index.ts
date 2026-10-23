/**
 * The dialog kit (ADR 0054, Paddock ADR 0009): kinds, `defineDialog`, the contract types and the size/modality policy.
 * It imports only `@angular/core` and nothing of Debrief, so the folder moves to `@decryp7/paddock-ui` unchanged; the app
 * keeps its registry entries, backend groups and command lists (see `../dialog-def.ts`, `../dialog-registry.ts`).
 */
export * from './dialog-kind';
export * from './define-dialog';
