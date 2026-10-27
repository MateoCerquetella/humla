import { describe, it, expect } from "vitest";
import { computeSetupStatus } from "./setupStatus";
import { mockTauri } from "../test/tauri";

function withSummaryProvider(provider: string, keys: Record<string, string>) {
  const asked: string[] = [];
  mockTauri({
    settings_get: (args) => ((args as { key: string }).key === "summary_provider" ? provider : null),
    provider_key_get: (args) => {
      const p = (args as { provider: string }).provider;
      asked.push(p);
      return keys[p] ?? null;
    },
  });
  return asked;
}

describe("computeSetupStatus: summary configured", () => {
  it("reads the key of the summary provider in force, not OpenAI's", async () => {
    const asked = withSummaryProvider("anthropic", { anthropic: "stored" });
    expect((await computeSetupStatus()).summaryConfigured).toBe(true);
    expect(asked).not.toContain("openai");
  });

  it("is not configured when only another provider has a key", async () => {
    withSummaryProvider("anthropic", { openai: "stored" });
    expect((await computeSetupStatus()).summaryConfigured).toBe(false);
  });

  it("reads OpenAI's key when no provider is chosen", async () => {
    withSummaryProvider("", { openai: "stored" });
    expect((await computeSetupStatus()).summaryConfigured).toBe(true);
  });
});
