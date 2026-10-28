import { listen } from "@tauri-apps/api/event";
import { Pause, Play, Square } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { formatTime } from "../lib/captureClock";
import { ipc, type RecordingStatus } from "../lib/ipc";

// The floating bar's own view: the same recording as the window, reduced to the
// three things worth seeing from another app — that it is running, for how
// long, and how to stop it.
//
// Phase is read from `recording_status` rather than from the store, because
// this window mounts on its own and never runs `bindBackendListeners`.
export function FloatBar() {
  const [phase, setPhase] = useState<RecordingStatus["phase"]>("recording");
  const [elapsedMs, setElapsedMs] = useState(0);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    // Transparent window: the panel paints itself, the page behind it must not.
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";

    let stop: (() => void) | undefined;
    let cancelled = false;

    void ipc.recordingElapsedMs().then((ms) => {
      if (!cancelled) setElapsedMs(ms);
    });

    void listen<RecordingStatus>("recording_status", (event) => {
      setPhase(event.payload.phase);
      // A stop ends the clock; nothing renders past it, but freezing here keeps
      // the last frame honest if the window hasn't hidden yet.
      if (event.payload.phase === "recording") {
        void ipc.recordingElapsedMs().then((ms) => setElapsedMs(ms));
      }
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else stop = unlisten;
    });

    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  useEffect(() => {
    if (phase !== "recording") return;
    const id = window.setInterval(() => setElapsedMs((ms) => ms + 500), 500);
    return () => window.clearInterval(id);
  }, [phase]);

  const paused = phase === "paused";

  const run = async (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    try {
      await action();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      data-tauri-drag-region
      className="h-16 w-[340px] px-2 py-2 select-none"
    >
      <div
        data-tauri-drag-region
        className="flex h-full items-center gap-3 rounded-2xl border border-[var(--color-line-visible)] bg-[var(--color-surface)] px-4 shadow-xl"
      >
        <span
          className={
            "inline-block h-2.5 w-2.5 shrink-0 rounded-full " +
            (paused
              ? "bg-[var(--color-warning)]"
              : "animate-pulse bg-[var(--color-accent)]")
          }
        />
        <span
          data-tauri-drag-region
          className="font-mono text-sm tabular-nums text-[var(--color-text)]"
        >
          {paused ? "Paused" : formatTime(Math.floor(elapsedMs / 1000))}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          <BarButton
            label={paused ? "Resume recording" : "Pause recording"}
            disabled={busy}
            onClick={() =>
              run(paused ? ipc.recordingResume : ipc.recordingPause)
            }
          >
            {paused ? <Play size={14} /> : <Pause size={14} />}
          </BarButton>
          <BarButton
            label="Stop recording"
            disabled={busy}
            onClick={() => run(ipc.recordingStop)}
          >
            <Square size={11} fill="currentColor" />
          </BarButton>
        </div>
      </div>
    </div>
  );
}

function BarButton({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className="flex h-8 w-8 items-center justify-center rounded-full text-[var(--color-text-muted)] transition-colors hover:bg-[var(--color-accent-soft)] hover:text-[var(--color-text)] disabled:opacity-40"
    >
      {children}
    </button>
  );
}
