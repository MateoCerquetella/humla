import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/app";
import { makeNote } from "../test/fixtures";
import { mockLayoutBox } from "../test/layout";
import { useRecordingStore } from "../lib/store";
import type { TakeAudio } from "../lib/ipc";
import type { mockTauri } from "../test/tauri";

// ⋯ → Save audio… (#224): every take's stored audio, joined into one file and
// saved where the user picks, with the sheet listing what it would save first.

beforeAll(() => mockLayoutBox());
afterEach(() => {
  useRecordingStore.setState({ errors: [], transcribing: {} });
});

const take = (index: number, durationMs: number | null): TakeAudio => ({ index, durationMs });
const ALL_THERE = [take(1, 3_254_000), take(2, 240_000)];

// Each row of the sheet's list, label then value.
function rows(dialog: HTMLElement): string[][] {
  return within(dialog)
    .getAllByRole("listitem")
    .map((li) => Array.from(li.children).map((c) => c.textContent ?? ""));
}

function open(
  info: TakeAudio[] = ALL_THERE,
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
  it("lists what it would save, then saves it as M4A where the panel points", async () => {
    const saved = vi.fn();
    const panel = vi.fn((_args: unknown) => "/Users/me/Weekly planning.m4a");
    open(ALL_THERE, { "plugin:dialog|save": panel, export_note_audio: saved });

    const dialog = await openSheet();
    await waitFor(() =>
      expect(rows(dialog)).toEqual([
        ["Recording 1", "54 min 14 s"],
        ["Recording 2", "4 min"],
        ["Total", "58 min 14 s"],
      ]),
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

  it("marks a take whose audio is gone, and still saves the rest", async () => {
    open([take(1, 3_254_000), take(2, null), take(3, 240_000)]);
    const dialog = await openSheet();
    await waitFor(() =>
      expect(rows(dialog)).toEqual([
        ["Recording 1", "54 min 14 s"],
        ["Recording 2", "Missing"],
        ["Recording 3", "4 min"],
        ["Total", "58 min 14 s"],
      ]),
    );
    expect(screen.getByRole("button", { name: "Save…" })).toBeInTheDocument();
  });

  it("lists one take without a total", async () => {
    open([take(1, 38_000)]);
    const dialog = await openSheet();
    await waitFor(() => expect(rows(dialog)).toEqual([["Recording 1", "38 s"]]));
  });

  it("offers no save when every take's audio is gone, and says how to keep the next", async () => {
    open([take(1, null), take(2, null)], {}, false);
    const dialog = await openSheet();
    await waitFor(() =>
      expect(rows(dialog)).toEqual([
        ["Recording 1", "Missing"],
        ["Recording 2", "Missing"],
      ]),
    );
    expect(dialog).toHaveTextContent("Turn on Keep recorded audio in Settings → Recording");
    expect(screen.queryByRole("button", { name: "Save…" })).toBeNull();
    expect(screen.queryByRole("radiogroup")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("says so on a note with nothing recorded", async () => {
    open([]);
    const dialog = await openSheet();
    await waitFor(() => expect(dialog).toHaveTextContent("Nothing has been recorded on this note."));
    expect(screen.queryByRole("button", { name: "Save…" })).toBeNull();
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
