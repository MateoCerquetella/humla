import { listen } from "@tauri-apps/api/event";
import {
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
} from "@tauri-apps/api/window";
import { ChevronDown, ChevronUp, Pause, Play, Square } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { formatTime } from "../lib/captureClock";
import {
  ipc,
  type RecordingStatus,
  type TranscriptEvent,
} from "../lib/ipc";

// The floating bar's own view: the same recording as the window, reduced to
// what is worth seeing from another app — that it is running, for how long,
// the last thing that was said, and how to stop it.
//
// Phase and transcript are read from backend events rather than the store,
// because this window mounts on its own and never runs `bindBackendListeners`.
const EXPANDED = { width: 360, height: 140 };
const COLLAPSED = { width: 186, height: 60 };

// How much of the transcript the bar keeps. Two lines at this width is ~180
// characters; a little more is kept so a resize doesn't cut mid-sentence.
const CAPTION_CHARS = 300;

export function FloatBar() {
  const [phase, setPhase] = useState<RecordingStatus["phase"]>("recording");
  const [elapsedMs, setElapsedMs] = useState(0);
  const [caption, setCaption] = useState("");
  const [collapsed, setCollapsed] = useState(false);
  const [busy, setBusy] = useState(false);
  const sized = useRef<string | null>(null);

  useEffect(() => {
    // Transparent window: the panel paints itself, the page behind it must not.
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";

    let stop: (() => void)[] = [];
    let cancelled = false;

    void ipc.recordingElapsedMs().then((ms) => {
      if (!cancelled) setElapsedMs(ms);
    });

    void listen<RecordingStatus>("recording_status", (event) => {
      setPhase(event.payload.phase);
      if (event.payload.phase === "recording") {
        void ipc.recordingElapsedMs().then((ms) => setElapsedMs(ms));
      }
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else stop.push(unlisten);
    });

    // Live transcript: each chunk replaces the note's whole text, so the tail
    // of the latest replacement is the most recent speech.
    void listen<TranscriptEvent>("transcript_replaced", (event) => {
      const text = event.payload.text.trim();
      setCaption(text.slice(-CAPTION_CHARS));
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else stop.push(unlisten);
    });

    return () => {
      cancelled = true;
      for (const un of stop) un();
    };
  }, []);

  // The window is sized to its content: a transparent frame larger than the
  // pill would still swallow clicks meant for the app behind it.
  useEffect(() => {
    const next = collapsed ? COLLAPSED : EXPANDED;
    const key = `${next.width}x${next.height}`;
    if (sized.current === key) return;
    sized.current = key;

    void (async () => {
      const win = getCurrentWindow();
      try {
        const scale = await win.scaleFactor();
        const size = (await win.outerSize()).toLogical(scale);
        const pos = (await win.outerPosition()).toLogical(scale);
        await win.setSize(new LogicalSize(next.width, next.height));
        // Keep the bar's bottom edge and horizontal centre fixed, so growing
        // for the transcript pushes the panel up instead of off-screen.
        await win.setPosition(
          new LogicalPosition(
            pos.x + (size.width - next.width) / 2,
            pos.y + (size.height - next.height),
          ),
        );
      } catch (e) {
        console.error("floatbar resize failed", e);
      }
    })();
  }, [collapsed]);

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
    <div className="h-full w-full p-2 select-none">
      <div
        data-tauri-drag-region
        className="flex h-full flex-col overflow-hidden rounded-2xl border border-[var(--color-line-visible)] bg-[var(--color-surface)] shadow-xl"
      >
        <div
          data-tauri-drag-region
          className="flex h-14 shrink-0 items-center gap-3 px-4"
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
              label={collapsed ? "Expand" : "Minimise to a pill"}
              onClick={() => setCollapsed((c) => !c)}
            >
              {collapsed ? <ChevronUp size={15} /> : <ChevronDown size={15} />}
            </BarButton>
            {!collapsed && (
              <BarButton
                label={paused ? "Resume recording" : "Pause recording"}
                disabled={busy}
                onClick={() => run(paused ? ipc.recordingResume : ipc.recordingPause)}
              >
                {paused ? <Play size={14} /> : <Pause size={14} />}
              </BarButton>
            )}
            <BarButton
              label="Stop recording"
              disabled={busy}
              onClick={() => run(ipc.recordingStop)}
            >
              <Square size={11} fill="currentColor" />
            </BarButton>
          </div>
        </div>

        {!collapsed && (
          <div className="min-h-0 flex-1 border-t border-[var(--color-line-visible)] px-4 py-2">
            <p
              className={
                "line-clamp-2 text-xs leading-relaxed " +
                (caption
                  ? "text-[var(--color-text-muted)]"
                  : "italic text-[var(--color-text-disabled)]")
              }
            >
              {caption || (paused ? "Paused" : "Listening…")}
            </p>
          </div>
        )}
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
  disabled?: boolean;
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
