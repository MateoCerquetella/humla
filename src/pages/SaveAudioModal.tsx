import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { ipc, type AudioFormat, type Note, type NoteAudioInfo } from "../lib/ipc";
import { useRecordingStore } from "../lib/store";
import { formatDuration } from "../lib/sessions";
import { slug } from "./ExportModal";
import { Modal } from "./settings/components/Modal";
import { Segmented } from "./settings/components/Segmented";
import { Btn } from "./settings/components/Btn";

// ⋯ → Save audio… (#224). A note's audio lives under
// `recordings/<note id>/<take id>/playback.wav`, which nobody can be expected to
// find, so this saves it wherever the user picks: every take joined in order
// into one file, since what they want is "the meeting", not a folder of takes.
//
// Its own sheet rather than a checkbox in Export: Export writes one document
// through one save panel, and audio is a second file of a different kind.
//
// It says what it would save before it asks where, because a take's audio can
// be gone (retention off, "Delete stored audio…", a teammate's take that never
// downloaded) and a file quietly missing two of three takes reads as a bug.
const FORMATS: Record<AudioFormat, { label: string; filter: string }> = {
  m4a: { label: "M4A", filter: "M4A audio" },
  wav: { label: "WAV", filter: "WAV audio" },
};

function takes(n: number): string {
  return n === 1 ? "1 take" : `${n} takes`;
}

function whatItSaves(info: NoteAudioInfo): string {
  const length = formatDuration(info.durationMs);
  if (info.takesWithAudio === info.takes) {
    return info.takes === 1
      ? `${length} of audio.`
      : `${takes(info.takes)}, ${length} in all, saved as one file.`;
  }
  // Some takes kept their audio and some didn't.
  return `${info.takesWithAudio} of ${takes(info.takes)} still ${
    info.takesWithAudio === 1 ? "has its" : "have their"
  } audio on this Mac (${length}). The others aren’t stored, so the file leaves them out.`;
}

export function SaveAudioModal({
  note,
  open,
  onClose,
  keepAudio,
}: {
  note: Note;
  open: boolean;
  onClose: () => void;
  /** The device's retention setting, for why a note has no audio. */
  keepAudio: boolean;
}) {
  const pushError = useRecordingStore((s) => s.pushError);
  const [info, setInfo] = useState<NoteAudioInfo | null>(null);
  const [format, setFormat] = useState<AudioFormat>("m4a");
  const [busy, setBusy] = useState(false);

  // Asked on every open: a take can finish or lose its audio in between.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setInfo(null);
    ipc.noteAudioExportInfo(note.id).then(
      (i) => !cancelled && setInfo(i),
      () => !cancelled && setInfo({ takes: 0, takesWithAudio: 0, durationMs: 0 }),
    );
    return () => {
      cancelled = true;
    };
  }, [open, note.id]);

  const hasAudio = !!info && info.takesWithAudio > 0;

  async function onSave() {
    setBusy(true);
    try {
      const path = await save({
        defaultPath: `${slug(note.title)}.${format}`,
        filters: [{ name: FORMATS[format].filter, extensions: [format] }],
      });
      if (!path) return; // user cancelled the panel
      await ipc.exportNoteAudio(note.id, { path, format });
      onClose();
    } catch (e) {
      pushError({ noteId: note.id, message: String(e), kind: "export" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Save audio" size="sm">
      <div className="flex flex-col gap-4">
        <p className="text-sm">
          {!info
            ? "…"
            : hasAudio
              ? whatItSaves(info)
              : keepAudio
                ? "No audio is stored for this note."
                : "Audio isn’t kept on this Mac. Turn on Keep recorded audio in Settings → Recording to save it from your next recording."}
        </p>

        {hasAudio && (
          <div className="flex items-center justify-between gap-4">
            <span className="text-sm">Format</span>
            <Segmented<AudioFormat>
              label="Audio format"
              value={format}
              onChange={setFormat}
              options={(Object.keys(FORMATS) as AudioFormat[]).map((value) => ({
                value,
                label: FORMATS[value].label,
              }))}
            />
          </div>
        )}

        {/* Not until the answer is in: the sheet moves focus to its first
            control on open, and a lone Close standing in for the row would keep
            it once Save… and the format appear. */}
        {info && (
          <div className="flex justify-end gap-2 pt-1">
            <Btn onClick={onClose}>{hasAudio ? "Cancel" : "Close"}</Btn>
            {hasAudio && (
              <Btn onClick={onSave} disabled={busy}>
                {busy ? "Saving…" : "Save…"}
              </Btn>
            )}
          </div>
        )}
      </div>
    </Modal>
  );
}
