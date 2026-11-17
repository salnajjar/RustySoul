import {
  cleanup,
  render,
  screen,
  fireEvent,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import ApiAccess from "./ApiAccess";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("configures access, creates a scoped expiring key shown once and revokes it", async () => {
  let view = { enabled: false, keys: [] as object[], maximum_keys: 32 };
  const requests: Record<string, unknown>[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (_url, options) => {
    if (options?.method === "POST") {
      const q = JSON.parse(options.body as string);
      requests.push(q);
      if (q.action === "set_enabled") view = { ...view, enabled: q.enabled };
      if (q.action === "create") {
        const key = {
          id: "key-id",
          name: q.name,
          scope: q.scope,
          created_at: 1700000000,
          expires_at: 1707776000,
          last_used_at: null,
        };
        view = { ...view, keys: [key] };
        return new Response(
          JSON.stringify({ key, token: "rsapi_secret_once" }),
        );
      }
      if (q.action === "revoke") view = { ...view, keys: [] };
    }
    return new Response(JSON.stringify(view));
  });
  render(<ApiAccess token="" />);
  await screen.findByText("No keys have been created.");
  expect(screen.getByRole("button", { name: "Create API key" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Current administrator password"), {
    target: { value: "eightpwd" },
  });
  fireEvent.click(screen.getByLabelText("Enable external API access"));
  await waitFor(() =>
    expect(screen.getByLabelText("Enable external API access")).toBeChecked(),
  );
  fireEvent.change(screen.getByLabelText("Key name"), {
    target: { value: "Monitoring" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Create API key" }));
  expect(await screen.findByLabelText("New API key")).toHaveValue(
    "rsapi_secret_once",
  );
  expect(requests[1]).toEqual({
    action: "create",
    name: "Monitoring",
    scope: "read_only",
    expires_in_days: 90,
    current_password: "eightpwd",
  });
  expect(screen.getByLabelText("Current administrator password")).toHaveValue(
    "",
  );
  fireEvent.click(screen.getByRole("button", { name: "I saved this key" }));
  expect(screen.queryByLabelText("New API key")).not.toBeInTheDocument();
  fireEvent.click(
    screen.getByRole("button", { name: "Revoke key Monitoring" }),
  );
  await screen.findByText("No keys have been created.");
  expect(requests[2]).toEqual({ action: "revoke", id: "key-id" });
});
it("keeps API failures dismissible without exposing stored secrets", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(JSON.stringify({ error: "Current password is incorrect" }), {
      status: 400,
    }),
  );
  render(<ApiAccess token="" />);
  await screen.findByRole("alert");
  fireEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("restores the disabled switch if enabling is rejected", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async (_url, options) =>
    options?.method === "POST"
      ? new Response(
          JSON.stringify({ error: "Current password is incorrect" }),
          { status: 400 },
        )
      : new Response(
          JSON.stringify({ enabled: false, keys: [], maximum_keys: 32 }),
        ),
  );
  render(<ApiAccess token="" />);
  await screen.findByText("No keys have been created.");
  fireEvent.change(screen.getByLabelText("Current administrator password"), {
    target: { value: "wrong" },
  });
  fireEvent.click(screen.getByLabelText("Enable external API access"));
  await screen.findByRole("alert");
  expect(screen.getByLabelText("Enable external API access")).not.toBeChecked();
});
