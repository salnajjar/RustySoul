import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { RoomJoin } from "./RoomTools";
import { emptySnapshot, type Snapshot, type RoomSummary } from "./types";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
const initial = (): Snapshot => ({
  ...emptySnapshot,
  connection: "connected",
  username: "owner",
});
const roomInfo = (
  error: string | null,
  pending: string | null,
  revision: number,
): RoomSummary => ({
  private: true,
  owner: null,
  member: false,
  operator: false,
  confirmed: false,
  owner_live: false,
  members_known: false,
  operators_known: false,
  board_known: false,
  members: 0,
  operators: 0,
  board: 0,
  own_ticker: "",
  own_ticker_set: false,
  updated_at: 0,
  revision,
  pending,
  error,
});
function roomState(info: RoomSummary): Snapshot {
  return {
    ...initial(),
    room_management: {
      rooms: { Club: info },
      accept_invitations: false,
      invitations_confirmed: null,
      invitations_error: null,
    },
  };
}
async function submit() {
  fireEvent.change(screen.getByLabelText("Room name to join"), {
    target: { value: "Club" },
  });
  fireEvent.change(screen.getByLabelText("Room type"), {
    target: { value: "private" },
  });
  await act(async () => {
    fireEvent.click(
      screen.getByRole("button", { name: "Create or join private room" }),
    );
  });
}
it("waits for actual membership then opens the created room automatically", async () => {
  const send = vi.fn().mockResolvedValue(true),
    choose = vi.fn();
  const { rerender } = render(
    <RoomJoin state={initial()} send={send} choose={choose} />,
  );
  await submit();
  expect(send).toHaveBeenCalledWith({
    type: "create_private_room",
    room: "Club",
  });
  expect(choose).not.toHaveBeenCalled();
  expect(
    screen.getByRole("button", { name: "Waiting for room confirmation…" }),
  ).toBeDisabled();
  rerender(
    <RoomJoin
      state={roomState(roomInfo(null, "Creating private room…", 1))}
      send={send}
      choose={choose}
    />,
  );
  expect(choose).not.toHaveBeenCalled();
  const state = roomState({
    ...roomInfo(null, null, 2),
    owner: "owner",
    member: true,
    confirmed: true,
    owner_live: true,
  });
  // Ownership/membership alone does not mean the chat has been joined.
  rerender(<RoomJoin state={state} send={send} choose={choose} />);
  expect(choose).not.toHaveBeenCalled();
  rerender(
    <RoomJoin
      state={{ ...state, joined_rooms: ["Club"] }}
      send={send}
      choose={choose}
    />,
  );
  expect(choose).toHaveBeenCalledExactlyOnceWith("Club");
});
it("ignores a previous error while retrying and shows the new server error", async () => {
  const send = vi.fn().mockResolvedValue(true),
    choose = vi.fn();
  const stale = roomState(roomInfo("Old timeout", null, 3));
  const { rerender } = render(
    <RoomJoin state={stale} send={send} choose={choose} />,
  );
  await submit();
  expect(
    screen.getByRole("button", { name: "Waiting for room confirmation…" }),
  ).toBeDisabled();
  rerender(
    <RoomJoin
      state={roomState(roomInfo("This name is already a public room", null, 4))}
      send={send}
      choose={choose}
    />,
  );
  expect(screen.getByText("This name is already a public room")).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Create or join private room" }),
  ).toBeEnabled();
  expect(choose).not.toHaveBeenCalled();
});
it("returns a missing confirmation to an actionable error and permits retry", async () => {
  vi.useFakeTimers();
  const choose = vi.fn();
  render(
    <RoomJoin
      state={initial()}
      send={vi.fn().mockResolvedValue(true)}
      choose={choose}
    />,
  );
  await submit();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(25000);
  });
  expect(screen.getByText(/Check the room name and your access/)).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Create or join private room" }),
  ).toBeEnabled();
  expect(choose).not.toHaveBeenCalled();
});
