import { useEffect, useState, useCallback } from "react";

/**
 * Run `fn` once, `ms` after the last `call()`: a burst of calls fires it once, when the burst has
 * settled. `cancel()` drops a pending run. With `ms = 0` every call runs `fn` at once.
 */
export function settle(fn: () => void, ms: number): { call(): void; cancel(): void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const cancel = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  return {
    call() {
      if (ms <= 0) return fn();
      cancel();
      timer = setTimeout(() => {
        timer = undefined;
        fn();
      }, ms);
    },
    cancel,
  };
}

/**
 * Subscribe to an SSE endpoint and return a counter that increments on every "changed" event.
 * Components use it as a dependency to refetch data live as the CLI edits the data folder. No auth
 * (the view daemon is a localhost read-only view), so the native EventSource is enough.
 *
 * With `settleMs > 0` a burst of events bumps the counter once, `settleMs` after the last one, for
 * a page whose refetch is costly (DEF-022).
 */
export function useLiveTick(sseUrl: string, settleMs = 0): number {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const es = new EventSource(sseUrl);
    const bump = settle(() => setTick((t) => t + 1), settleMs);
    es.addEventListener("changed", bump.call);
    es.onerror = () => {
      /* EventSource auto-reconnects; ignore transient errors */
    };
    return () => {
      bump.cancel();
      es.close();
    };
  }, [sseUrl, settleMs]);
  return tick;
}

interface AsyncState<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
}

/** Fetch data, re-running whenever `deps` change (e.g. the live tick). */
export function useAsync<T>(fn: () => Promise<T>, deps: unknown[]): AsyncState<T> & { reload: () => void } {
  const [state, setState] = useState<AsyncState<T>>({ data: null, error: null, loading: true });
  const run = useCallback(() => {
    let cancelled = false;
    setState((s) => ({ ...s, loading: true }));
    fn()
      .then((data) => !cancelled && setState({ data, error: null, loading: false }))
      .catch((e) => !cancelled && setState({ data: null, error: String(e), loading: false }));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  useEffect(run, [run]);
  return { ...state, reload: run };
}
