import { useState, useCallback, useEffect, useRef, useMemo } from "react";

const MAX_RETRIES = 3;
const RETRY_DELAYS = [200, 600, 1500] as const;

export function useImageRetry(url: string | null | undefined) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [retryCount, setRetryCount] = useState(0);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (url) {
      setLoading(true);
      setError(false);
      setRetryCount(0);
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current);
        retryTimerRef.current = null;
      }
    }
  }, [url]);

  useEffect(() => {
    return () => {
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current);
    };
  }, []);

  const src = useMemo(() => {
    if (!url) return null;
    if (retryCount === 0) return url;
    const sep = url.includes("?") ? "&" : "?";
    return `${url}${sep}_retry=${retryCount}&t=${Date.now()}`;
  }, [url, retryCount]);

  const onLoad = useCallback(() => {
    setLoading(false);
    setError(false);
  }, []);

  const onError = useCallback(() => {
    if (retryTimerRef.current) return;
    if (retryCount < MAX_RETRIES) {
      const delay = RETRY_DELAYS[retryCount] ?? 1500;
      retryTimerRef.current = setTimeout(() => {
        retryTimerRef.current = null;
        setRetryCount((c) => c + 1);
      }, delay);
    } else {
      setLoading(false);
      setError(true);
    }
  }, [retryCount]);

  const retry = useCallback(() => {
    setRetryCount(0);
    setError(false);
    setLoading(true);
    requestAnimationFrame(() => setRetryCount(1));
  }, []);

  return { src, loading, error, onLoad, onError, retry };
}
