import { useEffect, useState, useCallback } from "react";

/**
 * Subscribe to an SSE endpoint and return a counter that increments on every "changed" event.
 * Components use it as a dependency to refetch data live as the CLI edits the data folder. No auth
 * (the view daemon is a localhost read-only view), so the native EventSource is enough.
 */
export function useLiveTick(sseUrl: string): number {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const es = new EventSource(sseUrl);
    const bump = () => setTick((t) => t + 1);
    es.addEventListener("changed", bump);
    es.onerror = () => {
      /* EventSource auto-reconnects; ignore transient errors */
    };
    return () => es.close();
  }, [sseUrl]);
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
