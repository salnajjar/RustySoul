import { describe, expect, it, vi, afterEach } from "vitest";
import { api, basename, bytes, subscribe } from "./api";
afterEach(() => vi.restoreAllMocks());
describe("formatting", () => {
  it("formats sizes at boundaries and handles invalid data", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(-5)).toBe("0 B");
    expect(bytes(NaN)).toBe("0 B");
    expect(bytes(1024)).toBe("1 KiB");
    expect(bytes(1024 ** 3)).toBe("1 GiB");
  });
  it("handles Soulseek and Unix separators", () => {
    expect(basename("Music\\Album\\track.flac")).toBe("track.flac");
    expect(basename("Music/Album/track.flac")).toBe("track.flac");
  });
});
describe("authenticated transport", () => {
  it("puts credentials in a header only", async () => {
    const fetcher = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response('{"ok":true}'));
    await api("secret", "commands", { type: "connect" });
    expect(fetcher.mock.calls[0][0]).toBe("/api/v1/commands");
    expect(fetcher.mock.calls[0][1]?.headers).toMatchObject({
      Authorization: "Bearer secret",
    });
    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
  });
  it("surfaces server errors", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response('{"error":"Denied"}', { status: 401 }),
    );
    await expect(api("wrong", "status")).rejects.toThrow("Denied");
  });
  it("decodes a state event split across network chunks", async () => {
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        for (const part of [
          "event: state\nda",
          'ta: {"connection":',
          '"connected"}\n\n: heartbeat\n\n',
        ])
          controller.enqueue(encoder.encode(part));
        controller.close();
      },
    });
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(stream));
    const update = vi.fn();
    await expect(
      subscribe("secret", update, new AbortController().signal),
    ).rejects.toThrow("closed");
    expect(update).toHaveBeenCalledExactlyOnceWith({ connection: "connected" });
  });
});
