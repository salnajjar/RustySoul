import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import StorageReserve from "./StorageReserve";
import { emptySnapshot } from "./types";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("defaults older settings to 1 GiB and only submits the reserve without overwriting network preferences", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response("{}"));
  render(
    <StorageReserve
      state={{
        ...emptySnapshot,
        network: {
          ...emptySnapshot.network,
          minimum_free_space_mib: undefined,
        },
      }}
      token="session"
    />,
  );
  expect(
    screen.getByLabelText("Download free-space reserve (GiB)"),
  ).toHaveValue(1);
  fireEvent.change(screen.getByLabelText("Download free-space reserve (GiB)"), {
    target: { value: "0.5" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Save storage protection" }),
  );
  await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
  expect(JSON.parse(fetcher.mock.calls[0][1]!.body as string)).toEqual({
    type: "update_storage_reserve",
    minimum_free_space_mib: 512,
  });
});
it("supports disabling, rejects empty input and reports a dismissible server failure", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(
      JSON.stringify({ error: "Storage policy could not be saved" }),
      { status: 500 },
    ),
  );
  render(
    <StorageReserve
      state={{
        ...emptySnapshot,
        network: { ...emptySnapshot.network, minimum_free_space_mib: 0 },
      }}
      token="session"
    />,
  );
  expect(
    screen.getByText("Download storage protection is disabled."),
  ).toBeVisible();
  const input = screen.getByLabelText("Download free-space reserve (GiB)");
  fireEvent.change(input, { target: { value: "" } });
  expect(
    screen.getByRole("button", { name: "Save storage protection" }),
  ).toBeDisabled();
  fireEvent.change(input, { target: { value: "0" } });
  fireEvent.click(
    screen.getByRole("button", { name: "Save storage protection" }),
  );
  expect(
    await screen.findByText("Storage policy could not be saved"),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));
  expect(
    screen.queryByText("Storage policy could not be saved"),
  ).not.toBeInTheDocument();
});
it("updates saved values from snapshots", () => {
  const view = render(<StorageReserve state={emptySnapshot} token="session" />);
  view.rerender(
    <StorageReserve
      state={{
        ...emptySnapshot,
        network: { ...emptySnapshot.network, minimum_free_space_mib: 2560 },
      }}
      token="session"
    />,
  );
  expect(
    screen.getByLabelText("Download free-space reserve (GiB)"),
  ).toHaveValue(2.5);
});
