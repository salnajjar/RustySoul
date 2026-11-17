import {
  fireEvent,
  render,
  screen,
  cleanup,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { emptySnapshot } from "./types";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
describe("workspace access", () => {
  it("starts locked with no populated or fictional network data", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response("{}", { status: 401 }),
    );
    render(<App />);
    await screen.findByLabelText("Username");
    expect(screen.getByLabelText("Username")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeDisabled();
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
  });
  it("shows invalid-token errors and remains locked", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async () =>
        new Response('{"error":"Invalid username or password"}', {
          status: 401,
        }),
    );
    render(<App />);
    await screen.findByLabelText("Username");
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "wrong" },
    });
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "test-password" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Invalid username or password",
    );
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
  });
  it("unlocks the workspace and can clear the token by locking it", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async (url, options) => {
      if (String(url).endsWith("/events"))
        return new Promise((_resolve, reject) =>
          options?.signal?.addEventListener("abort", () =>
            reject(new Error("aborted")),
          ),
        );
      if (String(url).endsWith("auth/login"))
        return new Response(
          JSON.stringify({
            token: "session",
            account: { username: "fixture", must_change: false },
          }),
        );
      return new Response(JSON.stringify(emptySnapshot));
    });
    render(<App />);
    await screen.findByLabelText("Username");
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "s".repeat(32) },
    });
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "test-password" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() =>
      expect(screen.getByRole("navigation")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    expect(await screen.findByLabelText("Username")).toHaveValue("");
    expect(Object.keys(localStorage)).toEqual(["rustysoul.preferences"]);
    expect(localStorage.getItem("rustysoul.preferences")).not.toContain(
      "session",
    );
  });
});
