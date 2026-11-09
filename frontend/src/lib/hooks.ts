"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { api, errorMessage } from "./api";
import { type Period, fromISODate, periodRange, toISODate } from "./time";

type Query = Record<string, string | undefined>;

interface FetchState<T> {
  key: string | null;
  data?: T;
  error?: string;
}

/**
 * GETs `path` (with optional `query`) and refetches whenever either changes.
 *
 * Loading is derived by comparing the key of the last response with the current
 * request key, so no state is set synchronously inside the effect.
 * `reload()` refetches in the background, keeping the current data on screen;
 * `mutate()` applies a local update (e.g. after a create or delete).
 */
export function useApiData<T>(path: string, query?: Query) {
  const queryKey = JSON.stringify(query ?? {});
  const key = `${path}?${queryKey}`;
  const [version, setVersion] = useState(0);
  const [state, setState] = useState<FetchState<T>>({ key: null });

  useEffect(() => {
    let cancelled = false;
    const requestKey = `${path}?${queryKey}`;
    api<T>(path, { query: JSON.parse(queryKey) as Query }).then(
      (data) => {
        if (!cancelled) setState({ key: requestKey, data });
      },
      (err: unknown) => {
        if (!cancelled) setState({ key: requestKey, error: errorMessage(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [path, queryKey, version]);

  const reload = useCallback(() => setVersion((v) => v + 1), []);
  const mutate = useCallback(
    (fn: (prev: T) => T) =>
      setState((s) => (s.data === undefined ? s : { ...s, data: fn(s.data) })),
    [],
  );

  const current = state.key === key;
  return {
    data: current ? state.data : undefined,
    error: current ? state.error : undefined,
    loading: !current,
    reload,
    mutate,
  };
}

/** Day / week / month period state shared by every report page. */
export function usePeriod(initial: Period) {
  const [period, setPeriod] = useState<Period>(initial);
  // Stored as YYYY-MM-DD so the derived range is stable across renders.
  const [anchorKey, setAnchorKey] = useState(() => toISODate(new Date()));
  const anchor = useMemo(() => fromISODate(anchorKey), [anchorKey]);
  const { from, to } = useMemo(() => periodRange(period, anchor), [period, anchor]);
  const setAnchor = useCallback((d: Date) => setAnchorKey(toISODate(d)), []);

  return {
    period,
    setPeriod,
    anchor,
    setAnchor,
    from,
    to,
    fromKey: toISODate(from),
    toKey: toISODate(to),
  };
}
