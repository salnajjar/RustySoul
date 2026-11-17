import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { it, expect, afterEach, vi } from "vitest";
import App from "./App";
import { AccountForm } from "./Access";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("forces replacement before rendering any workspace navigation", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async () =>
      new Response(
        JSON.stringify({
          token: "setup-session",
          account: { username: "admin", must_change: true },
        }),
      ),
  );
  render(<App />);
  await screen.findByLabelText("Username");
  fireEvent.change(screen.getByLabelText("Username"), {
    target: { value: "admin" },
  });
  fireEvent.change(screen.getByLabelText("Password"), {
    target: { value: "change-me-now" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
  expect(
    await screen.findByRole("heading", {
      name: "Secure your administrator account",
    }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
  expect(screen.queryByText("Soulseek account")).not.toBeInTheDocument();
});
it("rejects mismatched confirmation without sending a request", () => {
  const fetcher = vi.spyOn(globalThis, "fetch");
  render(
    <AccountForm
      access={{
        token: "session",
        account: { username: "admin", must_change: true },
      }}
      onChange={vi.fn()}
    />,
  );
  fireEvent.change(screen.getByLabelText("Current password"), {
    target: { value: "change-me-now" },
  });
  fireEvent.change(screen.getByLabelText("New username"), {
    target: { value: "newadmin" },
  });
  fireEvent.change(screen.getByLabelText("New password"), {
    target: { value: "new-long-password" },
  });
  fireEvent.change(screen.getByLabelText("Confirm new password"), {
    target: { value: "different-password" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Save administrator account" }),
  );
  expect(screen.getByRole("alert")).toHaveTextContent("do not match");
  expect(fetcher).not.toHaveBeenCalled();
});
