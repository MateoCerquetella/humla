import { useCallback, useEffect, useState } from "react";
import { ipc, type CopilotStatus } from "../../lib/ipc";
import { copilotReady } from "../../lib/copilot";

export { copilotReady };

/** What still stands between Copilot and a working call, or `""` when nothing
 *  does. `at` says where the controls are ("above", "in Settings → Chat"). */
export function copilotHint(
  { status, error }: { status: CopilotStatus | null; error: string | null },
  at: string,
): string {
  if (!status) return error ? `Couldn't check GitHub Copilot: ${error}` : "Checking GitHub Copilot…";
  if (!status.cliPath) return `Download the Copilot CLI ${at} to use GitHub Copilot.`;
  if (!status.login) return `Sign in to GitHub Copilot ${at}.`;
  return "";
}

export type CopilotHook = ReturnType<typeof useCopilotStatus>;

/** Copilot's sign-in and CLI state. Inert while disabled: `copilot_status`
 *  reads the Keychain and may spawn a login shell, so it runs only while
 *  Copilot is the provider on screen. */
export function useCopilotStatus({ enabled }: { enabled: boolean }) {
  const [status, setStatus] = useState<CopilotStatus | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    if (!enabled) {
      setStatus(null);
      setLoading(false);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    ipc
      .copilotStatus()
      .then((s) => {
        if (cancelled) return;
        setStatus(s);
        setError(null);
      })
      .catch((e) => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [enabled, nonce]);

  const refresh = useCallback(() => setNonce((n) => n + 1), []);
  const setLogin = useCallback(
    (login: string | null) => setStatus((s) => (s ? { ...s, login } : s)),
    [],
  );

  return { status, loading, error, refresh, setLogin, ready: copilotReady(status) };
}

/** Options for a Copilot model picker. Listed only once Copilot is ready; until
 *  then, or if the listing fails, Auto stands in. The stored value is always
 *  among the options so the picker never reads blank. */
export function useCopilotModels(ready: boolean, selected: string) {
  const [live, setLive] = useState<{ value: string; label: string }[] | null>(null);

  useEffect(() => {
    setLive(null);
    if (!ready) return;
    let cancelled = false;
    ipc
      .copilotModels()
      .then((ms) => !cancelled && setLive(ms.length ? ms.map((m) => ({ value: m.id, label: m.name })) : null))
      .catch(() => !cancelled && setLive(null));
    return () => {
      cancelled = true;
    };
  }, [ready]);

  const options = live ?? [{ value: "auto", label: "Auto" }];
  return selected && !options.some((o) => o.value === selected)
    ? [{ value: selected, label: selected }, ...options]
    : options;
}
