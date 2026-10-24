import { MockClock, systemMockClock } from '../app/core/mock/mock-clock';

/**
 * A clock for the mock Pitwall: the wait for approval (the mock's 8 steps, so use `stepMs: 1`) lasts until `approve()`
 * is called, so a test can look at the pairing code for as long as it likes. Every other sleep is instant.
 */
export function gatedPairingClock(): { clock: MockClock; approve: () => void } {
  let release: () => void = () => undefined;
  const gate = new Promise<void>((resolve) => (release = resolve));
  return {
    clock: { ...systemMockClock, sleep: (ms) => (ms === 8 ? gate : Promise.resolve()) },
    approve: () => release(),
  };
}
