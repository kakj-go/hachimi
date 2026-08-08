import { describe, expect, it } from "vitest";

import { normalizeRemoteContextFields } from "./llm-settings-normalization";

describe("LLM settings normalization", () => {
  it("clears response-only fields for Chat Completions", () => {
    expect(normalizeRemoteContextFields("chat_completions", true, "auto", true)).toEqual({
      reasoningSummary: "none",
      remoteCompaction: false,
    });
  });

  it("preserves response fields when remote context is available", () => {
    expect(normalizeRemoteContextFields("responses", true, "detailed", true)).toEqual({
      reasoningSummary: "detailed",
      remoteCompaction: true,
    });
  });

  it("clears response fields when the feature is unavailable", () => {
    expect(normalizeRemoteContextFields("responses", false, "concise", true)).toEqual({
      reasoningSummary: "none",
      remoteCompaction: false,
    });
  });
});
