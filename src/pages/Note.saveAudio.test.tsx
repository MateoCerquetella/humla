import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/app";
import { makeNote } from "../test/fixtures";
import { mockLayoutBox } from "../test/layout";
import { useRecordingStore } from "../lib/store";
import type { NoteAudioInfo } from "../lib/ipc";
import type { mockTauri } from "../test/tauri";

// ⋯ → Save audio… (#224): every take's stored audio, joined into one file and
// saved where the user picks, with the sheet saying what it would save first.

beforeAll(() => mockLayoutBox());
afterEach(() => {
  useRecordingStore.setState({ errors: [], transcribing: {} });
});

const ALL_THERE: NoteAudioInfo = { takes: 2, takesWithAudio: 2, durationMs: 3_725_000 };

function open(
  info: NoteAudioInfo = ALL_THERE,
  handlers: Parameters<typeof mockTauri>[0] = {},
  keepAudio = true,
) {
  const note = makeNote({ id: "n1", title: "Weekly planning" });
  renderApp("/note/n1", {
    notes_list: () => [note],
    notes_get: () => note,
    settings_get: (args) => {
      const key = (args as { key: string }).key;
      if (key === "onboarding_completed") return "true";
      if (key === "keep_audio") return String(keepAudio);
      return null;
    },
    note_audio_export_info: () => info,
    ...handlers,
  });
}

const titleBox = () => screen.getByPlaceholderText("New note");

async function openSheet() {
  await waitFor(() => expect(titleBox()).toHaveValue("Weekly planning"));
  await userEvent.click(await screen.findByRole("button", { name: /more/i }));
  await userEvent.click(await screen.findByRole("menuitem", { name: "Save audio…" }));
  return screen.findByRole("dialog", { name: "Save audio" });
}

describe("⋯ → Save audio", () => {
  it("says what it would save, then saves it as M4A where the panel points", async () => {
    const saved = vi.fn();
    const panel = vi.fn((_args: unknown) => "/Users/me/Weekly planning.m4a");
    open(ALL_THERE, { "plugin:dialog|save": panel, export_note_audio: saved });

    const dialog = await openSheet();
    await waitFor(() =>
      expect(dialog).toHaveTextContent("2 takes, 1:02:05 in all, saved as one file."),
    );
    await userEvent.click(screen.getByRole("button", { name: "Save…" }));

    await waitFor(() =>
      expect(saved).toHaveBeenCalledWith(
        expect.objectContaining({
          noteId: "n1",
          spec: { path: "/Users/me/Weekly planning.m4a", format: "m4a" },
        }),
      ),
    );
    const options = (panel.mock.calls[0][0] as { options: { defaultPath: string } }).options;
    expect(options.defaultPath).toBe("weekly-planning.m4a");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("saves WAV when picked, and a cancelled panel saves nothing", async () => {
    const saved = vi.fn();
    let path: string | null = null;
    const panel = vi.fn((_args: unknown) => path);
    open(ALL_THERE, { "plugin:dialog|save": panel, export_note_audio: saved });

    await openSheet();
    await userEvent.click(await screen.findByRole("radio", { name: "WAV" }));
    await userEvent.click(screen.getByRole("button", { name: "Save…" }));
    await waitFor(() => expect(panel).toHaveBeenCalledTimes(1));
    expect(saved).not.toHaveBeenCalled();
    // Still open, to pick again.
    expect(screen.getByRole("dialog", { name: "Save audio" })).toBeInTheDocument();

    path = "/Users/me/planning.wav";
    await userEvent.click(screen.getByRole("button", { name: "Save…" }));
    await waitFor(() =>
      expect(saved).toHaveBeenCalledWith(
        expect.objectContaining({ spec: { path: "/Users/me/planning.wav", format: "wav" } }),
      ),
    );
    const options = (panel.mock.calls[1][0] as { options: { defaultPath: string } }).options;
    expect(options.defaultPath).toBe("weekly-planning.wav");
  });

  it("says when some takes no longer have their audio", async () => {
    open({ takes: 3, takesWithAudio: 1, durationMs: 65_000 });
    const dialog = await openSheet();
    await waitFor(() =>
      expect(dialog).toHaveTextContent(
        "1 of 3 takes still has its audio on this Mac (1:05). The others aren't stored, so the file leaves them out.",
      ),
    );
    expect(screen.getByRole("button", { name: "Save…" })).toBeInTheDocument();
  });

  it("explains a note with no audio instead of offering to save one", async () => {
    open({ takes: 1, takesWithAudio: 0, durationMs: 0 }, {}, false);
    const dialog = await openSheet();
    await waitFor(() =>
      expect(dialog).toHaveTextContent(
        "Audio isn't kept on this Mac. Turn on Keep recorded audio in Settings → Recording",
      ),
    );
    expect(screen.queryByRole("button", { name: "Save…" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("surfaces a failed save as an error", async () => {
    open(ALL_THERE, {
      "plugin:dialog|save": () => "/Users/me/x.m4a",
      export_note_audio: () => {
        throw new Error("convert audio to M4A: boom");
      },
    });
    await openSheet();
    await userEvent.click(await screen.findByRole("button", { name: "Save…" }));
    await waitFor(() =>
      expect(useRecordingStore.getState().errors).toEqual([
        expect.objectContaining({ noteId: "n1", kind: "export", message: expect.stringContaining("boom") }),
      ]),
    );
  });

  it("is not offered while a take is still being transcribed", async () => {
    open();
    await waitFor(() => expect(titleBox()).toHaveValue("Weekly planning"));
    act(() =>
      useRecordingStore.setState({
        transcribing: { n1: {} as ReturnType<typeof useRecordingStore.getState>["transcribing"][string] },
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: /more/i }));
    const item = await screen.findByRole("menuitem", { name: "Save audio…" });
    expect(item).toHaveAttribute("data-disabled");
  });
});
