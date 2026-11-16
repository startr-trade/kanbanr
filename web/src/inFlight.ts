// One call at a time for a button (DEF-090). A second press while the first call is still in
// flight is dropped, never queued, so a double click sends one request. Pure, so the panel and
// web::check_prepared_panel_sends_one_request_per_click_until_it_returns
// (scripts/check-review-outcome.mjs) use the same code.

/**
 * Wrap `fn` so that `run` calls it only when no call is in flight. A call is in flight from `run`
 * until the promise `fn` returned settles; a synchronous `fn` counts as settled at once. `run`
 * swallows nothing: the caller's handler turns its own errors into an outcome.
 */
export function guardInFlight<A extends unknown[]>(
  fn: (...a: A) => unknown,
): { run: (...a: A) => Promise<void>; busy: () => boolean } {
  let inFlight = false;
  return {
    run: async (...a: A) => {
      if (inFlight) return;
      inFlight = true;
      let result: unknown;
      try {
        result = fn(...a);
      } catch (e) {
        inFlight = false;
        throw e;
      }
      if (result == null || typeof (result as PromiseLike<unknown>).then !== "function") {
        inFlight = false;
        return;
      }
      try {
        await result;
      } finally {
        inFlight = false;
      }
    },
    busy: () => inFlight,
  };
}
