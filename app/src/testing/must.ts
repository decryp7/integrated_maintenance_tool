/**
 * Test-only accessors for `noUncheckedIndexedAccess`: they narrow away `undefined` and fail the test
 * with a clear message (instead of a later TypeError) when the expected value is missing.
 */

/** Returns `value`, failing the test when it is `null` or `undefined`. */
export function must<T>(value: T | null | undefined, what = 'value'): T {
  if (value === null || value === undefined) throw new Error(`expected ${what} to be present`);
  return value;
}

/** Returns `items[index]`, failing the test when there is no such element. */
export function at<T>(items: ArrayLike<T>, index: number): T {
  const i = index < 0 ? items.length + index : index;
  if (i < 0 || i >= items.length)
    throw new Error(`expected an element at index ${index} (length ${items.length})`);
  return items[i] as T;
}
