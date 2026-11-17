import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import Backups from "./Backups";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
const summary = {
  administrator: "saved-admin",
  created_at: 1700000000,
  database_bytes: 400000,
  schema: 11,
  downloads: 77,
  uploads: 23,
  indexed_files: 94064,
  locations: { downloads: "/downloads", shares: ["/library"] },
};
it("creates a password-confirmed encrypted download without buffering it as a browser blob", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input, options) =>
      String(input).endsWith("/backups") && options?.method === "POST"
        ? new Response(
            JSON.stringify({
              id: "export-id",
              kind: "export",
              bytes: 400100,
              can_restart: true,
              summary,
            }),
          )
        : new Response(
            JSON.stringify({ busy: false, ready: null, can_restart: true }),
          ),
    );
  render(<Backups token="session" onRestored={vi.fn()} />);
  fireEvent.change(
    screen.getByLabelText("Current administrator password for backup"),
    { target: { value: "admin-password" } },
  );
  fireEvent.change(screen.getByLabelText("New backup passphrase"), {
    target: { value: "backup-password" },
  });
  fireEvent.change(screen.getByLabelText("Confirm backup passphrase"), {
    target: { value: "backup-password" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Create encrypted backup" }),
  );
  expect(
    await screen.findByRole("link", { name: "Download encrypted backup" }),
  ).toHaveAttribute("href", "/api/v1/backups/export-id/file");
  const call = fetcher.mock.calls.find((c) => c[1]?.method === "POST")!;
  expect(JSON.parse(call[1]!.body as string)).toEqual({
    current_password: "admin-password",
    passphrase: "backup-password",
  });
  expect(screen.getByLabelText("New backup passphrase")).toHaveValue("");
});
it("validates before offering restore and requires current credentials plus typed confirmation", async () => {
  let ready: unknown = null;
  const restore = vi.fn();
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, options) => {
    const url = String(input);
    if (url.endsWith("/backups/inspect")) {
      const form = options!.body as FormData;
      expect(form.get("passphrase")).toBe("backup-password");
      expect(form.get("file")).toBeInstanceOf(File);
      ready = {
        id: "restore-id",
        kind: "restore",
        bytes: 400000,
        summary,
        can_restart: true,
      };
      return new Response(JSON.stringify(ready));
    }
    if (url.endsWith("/restore")) {
      restore(JSON.parse(options!.body as string));
      return new Response(JSON.stringify({ status: "restart_requested" }));
    }
    return new Response(
      JSON.stringify({ busy: false, ready, can_restart: true }),
    );
  });
  render(<Backups token="session" onRestored={vi.fn()} />);
  expect(
    screen.queryByRole("button", { name: "Restore and restart" }),
  ).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Encrypted backup file"), {
    target: { files: [new File(["encrypted"], "saved.rsb")] },
  });
  fireEvent.change(screen.getByLabelText("Backup passphrase"), {
    target: { value: "backup-password" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Validate backup" }));
  await screen.findByRole("heading", { name: "Review before restoring" });
  expect(
    screen.getByRole("button", { name: "Restore and restart" }),
  ).toBeDisabled();
  fireEvent.change(
    screen.getByLabelText("Current administrator password for restore"),
    { target: { value: "current-password" } },
  );
  fireEvent.change(screen.getByLabelText("Type RESTORE to confirm"), {
    target: { value: "RESTORE" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Restore and restart" }));
  await screen.findByRole("heading", { name: "Restoring RustySoul" });
  expect(restore).toHaveBeenCalledWith({
    current_password: "current-password",
    confirmation: "RESTORE",
  });
});
it("hides stale validation when another file is selected and shows rejected restore recovery", async () => {
  const ready = {
    id: "restore-id",
    kind: "restore",
    bytes: 400000,
    summary,
    can_restart: true,
  };
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(
      JSON.stringify({
        busy: false,
        ready,
        can_restart: true,
        last_result: {
          id: "result",
          status: "rejected",
          message: "Previous application state recovered",
          at: 1,
        },
      }),
    ),
  );
  render(<Backups token="session" onRestored={vi.fn()} />);
  await screen.findByRole("heading", { name: "Review before restoring" });
  expect(
    screen.getByText("Previous application state recovered"),
  ).toBeVisible();
  fireEvent.change(screen.getByLabelText("Encrypted backup file"), {
    target: { files: [new File(["another"], "another.rsb")] },
  });
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Restore and restart" }),
    ).not.toBeInTheDocument(),
  );
});
