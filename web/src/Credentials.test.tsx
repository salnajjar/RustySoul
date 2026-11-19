import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
} from "@testing-library/react";
import { afterEach, it, expect, vi } from "vitest";
import Credentials from "./Credentials";
import { emptySnapshot } from "./types";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("creates a human-chosen account through verified login and checks confirmation first", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response('{"status":"queued"}', { status: 202 }));
  render(<Credentials token="token" state={emptySnapshot} />);
  fireEvent.click(
    screen.getByRole("button", { name: "Create a Soulseek account" }),
  );
  fireEvent.change(screen.getByLabelText("Soulseek username"), {
    target: { value: "my-chosen-name" },
  });
  fireEvent.change(
    screen.getByLabelText("Soulseek password", { exact: true }),
    { target: { value: "chosen-password" } },
  );
  fireEvent.change(screen.getByLabelText("Confirm Soulseek password"), {
    target: { value: "different-password" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Create account and connect" }),
  );
  expect(fetcher).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toHaveTextContent("same password twice");
  fireEvent.change(screen.getByLabelText("Confirm Soulseek password"), {
    target: { value: "chosen-password" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Create account and connect" }),
  );
  await waitFor(() => expect(fetcher).toHaveBeenCalledOnce());
  expect(JSON.parse(String(fetcher.mock.calls[0][1]?.body))).toEqual({
    type: "update_credentials",
    username: "my-chosen-name",
    password: "chosen-password",
  });
  expect(
    screen.getByLabelText("Soulseek password", { exact: true }),
  ).toHaveValue("chosen-password");
});
it("saves a password and clears it only after the daemon confirms persistence", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response('{"status":"queued"}', { status: 202 }));
  const { rerender } = render(
    <Credentials token="token" state={emptySnapshot} />,
  );
  fireEvent.change(screen.getByLabelText("Soulseek username"), {
    target: { value: "fixture" },
  });
  fireEvent.change(screen.getByLabelText("Soulseek password"), {
    target: { value: "private-password" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save account" }));
  await waitFor(() => expect(fetcher).toHaveBeenCalled());
  expect(JSON.parse(String(fetcher.mock.calls[0][1]?.body))).toEqual({
    type: "update_credentials",
    username: "fixture",
    password: "private-password",
  });
  rerender(
    <Credentials
      token="token"
      state={{
        ...emptySnapshot,
        username: "fixture",
        credentials_configured: true,
        credentials_revision: 1,
        connection: "connected",
      }}
    />,
  );
  expect(screen.getByLabelText("Soulseek password")).toHaveValue("");
  expect(screen.getByText(/Login verified. Account saved/)).toBeVisible();
});
it("prevents editing a connected account", () => {
  render(
    <Credentials
      token="token"
      state={{
        ...emptySnapshot,
        connection: "connected",
        username: "fixture",
        credentials_configured: true,
      }}
    />,
  );
  expect(screen.getByLabelText("Soulseek password")).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Remove saved account" }),
  ).toBeDisabled();
});
