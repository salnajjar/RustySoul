import { test, expect, type Page } from "@playwright/test";
import { emptySnapshot, type Snapshot, type RoomSummary } from "../src/types";

const roomSummary = (change: Partial<RoomSummary> = {}): RoomSummary => ({
  private: true,
  owner: "fixture-self",
  member: true,
  operator: false,
  confirmed: true,
  owner_live: true,
  members_known: true,
  operators_known: true,
  board_known: true,
  members: 3,
  operators: 1,
  board: 1,
  own_ticker: "Saved board",
  own_ticker_set: true,
  updated_at: 1700000000,
  revision: 1,
  pending: null,
  error: null,
  ...change,
});
const roomState = (summary = roomSummary()): Snapshot => ({
  ...emptySnapshot,
  connection: "connected",
  username: "fixture-self",
  rooms: [{ name: "Public", users: 10 }],
  joined_rooms: ["club"],
  subscribed_rooms: ["club"],
  room_members: { club: ["fixture-self", "alice", "bob"] },
  room_management: {
    accept_invitations: false,
    invitations_confirmed: false,
    invitations_error: null,
    rooms: { club: summary },
  },
});
async function pushRoomState(page: Page, state: Snapshot) {
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    state,
  );
}
async function roomEntryFixture(page: Page, count = 3) {
  const requests: URLSearchParams[] = [];
  await page.route("**/api/v1/rooms/entries?**", (r) => {
    const q = new URL(r.request().url()).searchParams;
    requests.push(q);
    const kind = q.get("kind"),
      offset = Number(q.get("offset")),
      filter = q.get("filter")?.toLowerCase() || "";
    const values =
      kind === "board"
        ? [
            {
              username: "alice",
              text: "<script>literal board</script>",
              operator: true,
            },
          ]
        : kind === "operator"
          ? [{ username: "alice", text: "", operator: true }]
          : Array.from({ length: count }, (_, n) => ({
              username:
                count === 3
                  ? ["fixture-self", "alice", "bob"][n]
                  : `user-${String(n).padStart(4, "0")}`,
              text: "",
              operator: count === 3 && n === 1,
            }));
    const filtered = values.filter((e) =>
      (e.username + e.text).toLowerCase().includes(filter),
    );
    return r.fulfill({
      json: {
        entries: filtered.slice(offset, offset + 100),
        total: filtered.length,
        offset,
      },
    });
  });
  return requests;
}
test("private rooms can be created explicitly and invitation and visibility controls are available", async ({
  page,
}) => {
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const state = roomState();
  await fixture(page, state, true);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await page.locator(".room-join-settings > summary").click();
  await page.getByLabel("Room name to join", { exact: true }).fill("My Club");
  await page.getByLabel("Room type", { exact: true }).selectOption("private");
  await page
    .getByRole("button", { name: "Create or join private room", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "create_private_room",
    room: "My Club",
  });
  await page
    .getByLabel("Allow private-room invitations", { exact: true })
    .check();
  expect(commands).toContainEqual({
    type: "set_room_invitations",
    enabled: true,
  });
  await page
    .getByLabel("Room visibility", { exact: true })
    .selectOption("private");
  await expect(page.locator(".room-list .room-item")).toHaveCount(1);
  await expect(page.locator(".room-list .room-item")).toContainText("club");
  await page
    .getByLabel("Room visibility", { exact: true })
    .selectOption("public");
  await expect(page.locator(".room-list .room-item")).toHaveCount(1);
  await expect(page.locator(".room-list .room-item")).toContainText("Public");
  await page.getByLabel("Order rooms", { exact: true }).selectOption("name");
  await page.reload();
  await expect(page.getByLabel("Order rooms", { exact: true })).toHaveValue(
    "name",
  );
});
test("room boards load lazily, render text safely and allow offline saved messages", async ({
  page,
}, info) => {
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const requests = await roomEntryFixture(page);
  const state = roomState();
  await fixture(page, state);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  expect(requests).toHaveLength(0);
  await page.getByRole("tab", { name: "Room board", exact: true }).click();
  await expect(
    page.getByRole("tabpanel", { name: "Room board for club", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".room-list")).toHaveCount(0);
  await expect(
    page.getByPlaceholder("Search board messages or usernames in this room…"),
  ).toBeVisible();
  await expect(
    page.getByRole("list", { name: "room-board messages", exact: true }),
  ).toContainText("<script>literal board</script>");
  expect(await page.locator(".room-entries script").count()).toBe(0);
  await page
    .getByLabel("My room-board message", { exact: true })
    .fill("A new board message");
  await page
    .getByRole("button", {
      name: "Save and publish board message",
      exact: true,
    })
    .click();
  expect(commands).toContainEqual({
    type: "set_room_ticker",
    room: "club",
    text: "A new board message",
  });
  await page
    .getByRole("button", { name: "Clear my board message", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "set_room_ticker",
    room: "club",
    text: "",
  });
  await pushRoomState(page, {
    ...state,
    connection: "disconnected",
    joined_rooms: [],
    room_management: {
      ...state.room_management!,
      rooms: {
        club: roomSummary({
          confirmed: false,
          owner_live: false,
          board_known: false,
        }),
      },
    },
  });
  await page
    .getByLabel("My room-board message", { exact: true })
    .fill("Publish on next join");
  await page
    .getByRole("button", { name: "Save board message", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "set_room_ticker",
    room: "club",
    text: "Publish on next join",
  });
  await expect(
    page.getByText(
      "Cached board. Join the room while connected to receive its current messages.",
      { exact: true },
    ),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `../artifacts/room-board-${info.project.name}.png`,
    fullPage: true,
  });
  const failed = {
    ...state,
    connection: "disconnected",
    joined_rooms: [],
    room_management: {
      ...state.room_management!,
      rooms: {
        club: roomSummary({
          confirmed: false,
          owner_live: false,
          board_known: false,
          error: "The server has not confirmed this board request.",
        }),
      },
    },
  };
  await pushRoomState(page, failed);
  await expect(page.locator(".room-tools").getByRole("alert")).toContainText(
    "has not confirmed",
  );
  await page
    .locator(".room-tools")
    .getByRole("button", { name: "Dismiss error", exact: true })
    .click();
  expect(commands).toContainEqual({ type: "dismiss_messages" });
  await expect(page.locator(".room-tools").getByRole("alert")).toHaveCount(0);
  await pushRoomState(page, {
    ...failed,
    room_management: {
      ...failed.room_management,
      rooms: {
        club: roomSummary({
          confirmed: false,
          owner_live: false,
          board_known: false,
        }),
      },
    },
  });
  for (const theme of ["light", "dark"]) {
    await page
      .getByRole("button", { name: "Open settings", exact: true })
      .click();
    await page.getByRole("tab", { name: "Appearance", exact: true }).click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page.getByRole("button", { name: "Rooms", exact: true }).click();
    await page.getByRole("tab", { name: "Room board", exact: true }).click();
    await expect(
      page.getByRole("list", { name: "room-board messages", exact: true }),
    ).toContainText("<script>literal board</script>");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `../artifacts/room-board-${theme}-${info.project.name}.png`,
      fullPage: true,
    });
  }
});

test("text emoticons display as emoji and native copy paste sends them back in room and private chat", async ({
  page,
  context,
}, info) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await page.route("**/api/v1/rooms/read", (r) =>
    r.fulfill({ status: 202, json: { status: "queued" } }),
  );
  const roomText =
    "Thanks :) :smile: :heart: :musical_note: 👩🏽‍🎤 <script>window.emojiRan=true</script>";
  const privateText =
    "Hello ;-) :thumbsup: :unknown: https://example.org/:smile:";
  const state = {
    ...roomState(),
    messages: [
      {
        id: 1,
        target: "club",
        username: "alice",
        text: roomText,
        private: false,
        timestamp: 1,
      },
      {
        id: 2,
        target: "alice",
        username: "alice",
        text: privateText,
        private: true,
        timestamp: 2,
      },
    ],
  };
  await fixture(page, state);
  const previousClipboard = await page.evaluate(() =>
    navigator.clipboard.readText(),
  );
  try {
    for (const [section, target, expected, privateChat] of [
      [
        "Rooms",
        "club",
        "Thanks 🙂 😄 ❤️ 🎵 👩🏽‍🎤 <script>window.emojiRan=true</script>",
        false,
      ],
      [
        "Messages",
        "alice",
        "Hello 😉 👍 :unknown: https://example.org/:smile:",
        true,
      ],
    ] as const) {
      await page.getByRole("button", { name: section, exact: true }).click();
      if (privateChat)
        await page
          .getByRole("button", {
            name: "Open conversation with alice",
            exact: true,
          })
          .click();
      const text = page.locator(".message-list .chat-message");
      await expect(text).toHaveText(expected);
      await expect(text.locator("script,img")).toHaveCount(0);
      await text.click();
      await text.evaluate((node) => {
        const range = document.createRange();
        range.selectNodeContents(node);
        const selection = getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.keyboard.press("Control+c");
      await expect
        .poll(() => page.evaluate(() => navigator.clipboard.readText()))
        .toBe(expected);
      const input = page.getByLabel("Message", { exact: true });
      await input.focus();
      await page.keyboard.press("Control+v");
      await expect(input).toHaveValue(expected);
      await page.getByRole("button", { name: "Send", exact: true }).click();
      await expect
        .poll(() => commands.filter((c) => c.type === "send_message").at(-1))
        .toEqual({
          type: "send_message",
          target,
          text: expected,
          private: privateChat,
        });
      await expect(input).toHaveValue("");
    }
    expect(
      await page.evaluate(
        () => (window as unknown as { emojiRan?: boolean }).emojiRan,
      ),
    ).toBeUndefined();
    await page.screenshot({
      path: `../artifacts/chat-emoticons-${info.project.name}.png`,
      fullPage: true,
    });
  } finally {
    await page.evaluate(
      (text) => navigator.clipboard.writeText(text),
      previousClipboard,
    );
  }
  expect(state.messages.map((m) => m.text)).toEqual([roomText, privateText]);
});

function statusFixture(): NonNullable<Snapshot["system"]> {
  return {
    sampled_at: Math.floor(Date.now() / 1000),
    uptime_seconds: 93784,
    process: {
      resident_bytes: 128 * 1024 ** 2,
      threads: 74,
      cpu_seconds: 125,
      cpu_percent: 12.5,
    },
    limits: {
      memory_bytes: 512 * 1024 ** 2,
      memory_used_bytes: 480 * 1024 ** 2,
      memory_scope: "self",
      tasks: 128,
      tasks_used: 77,
      tasks_scope: "self",
      cpu_cores: 2,
    },
    data: {
      path: "/var/lib/rustysoul",
      total_bytes: 100 * 1024 ** 3,
      used_bytes: 50 * 1024 ** 3,
      available_bytes: 45 * 1024 ** 3,
      error: null,
    },
    downloads: {
      path: "/downloads",
      total_bytes: 1000 * 1024 ** 3,
      used_bytes: 960 * 1024 ** 3,
      available_bytes: 30 * 1024 ** 3,
      error: null,
    },
    database_bytes: 32 * 1024 ** 2,
    wal_bytes: 4 * 1024 ** 2,
    download_transfers: {
      total: 316,
      states: {
        completed: 300,
        downloading: 5,
        waiting: 5,
        failed: 5,
        paused: 1,
      },
      speed: 615,
    },
    upload_transfers: {
      total: 12,
      states: { completed: 10, uploading: 1, blocked: 1 },
      speed: 8192,
    },
    process_error: null,
    database_error: null,
  };
}
test("system status updates live, covers all transfers and provides resource and storage diagnostics", async ({
  page,
}, info) => {
  let state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    system: statusFixture(),
    shares: { files: 94064, folders: 3000 },
    effective_peer_limit: 16,
    download_capacity_used: 11,
    transfer_capacity: 4096,
  };
  await page.route("**/api/v1/logs?**", (r) =>
    r.fulfill({ json: { entries: [], more: false } }),
  );
  await page.route("**/api/v1/commands", (r) =>
    r.fulfill({ status: 202, json: { status: "queued" } }),
  );
  await fixture(page, state, true);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "Status & privacy", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "System status", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("12.5%", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("meter", { name: "Cgroup memory usage", exact: true }),
  ).toHaveAttribute("value", "93.75");
  await expect(page.getByText("316", { exact: true })).toBeVisible();
  await expect(page.getByText("300", { exact: true })).toBeVisible();
  await expect(page.getByText(/Near the limit/)).toBeVisible();
  await expect(page.getByText(/Low free space/)).toBeVisible();
  state = {
    ...state,
    system: {
      ...state.system!,
      process: { ...state.system!.process, cpu_percent: 7.5 },
      limits: { ...state.system!.limits, memory_used_bytes: 200 * 1024 ** 2 },
      download_transfers: {
        ...state.system!.download_transfers!,
        total: 317,
        states: { ...state.system!.download_transfers!.states, completed: 301 },
      },
    },
  };
  await pushRoomState(page, state);
  await expect(page.getByText("7.5%", { exact: true })).toBeVisible();
  await expect(page.getByText("317", { exact: true })).toBeVisible();
  await expect(page.getByText(/Near the limit/)).toHaveCount(0);
  for (const theme of ["original", "light", "dark"]) {
    await page.getByRole("tab", { name: "Appearance", exact: true }).click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page
      .getByRole("tab", { name: "Status & privacy", exact: true })
      .click();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `../artifacts/system-status-${theme}-${info.project.name}.png`,
      fullPage: true,
    });
  }
  await page.locator(".system-monitoring > summary").click();
  await expect(
    page.getByRole("link", { name: "View metrics", exact: true }),
  ).toHaveAttribute("href", "/api/v1/metrics");
  await page
    .getByRole("button", { name: "View logs & activity", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Logs & activity", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("tab", { name: "Status & privacy", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Open network settings", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "Network settings", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await page
    .getByRole("tab", { name: "Status & privacy", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Open file locations", exact: true })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "File locations & local library",
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("tab", { name: "Status & privacy", exact: true })
    .click();
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "System status", exact: true }),
  ).toBeVisible();
});
test("system status marks stale and unavailable readings without inventing measurements", async ({
  page,
}) => {
  const status = statusFixture();
  status.sampled_at -= 40;
  status.downloads.available_bytes = null;
  status.downloads.error = "<script>window.statusRan=true</script>";
  status.download_transfers = null;
  status.database_error = "Statistics are temporarily unavailable";
  status.process.cpu_percent = null;
  status.process_error = "Process readings unavailable";
  await fixture(page, { ...emptySnapshot, system: status });
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "Status & privacy", exact: true })
    .click();
  await expect(
    page.getByText("System readings delayed", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/Capacity unavailable: <script>/)).toBeVisible();
  await expect(
    page.getByText("Transfer statistics are not available yet.", {
      exact: true,
    }),
  ).toBeVisible();
  expect(await page.locator(".system-status script").count()).toBe(0);
  expect(
    await page.evaluate(
      () => (window as unknown as { statusRan?: boolean }).statusRan,
    ),
  ).toBeUndefined();
  await pushRoomState(page, { ...emptySnapshot, system: undefined });
  await expect(
    page.getByText("Waiting for the first sample", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("meter", { name: "Cgroup memory usage", exact: true }),
  ).toHaveCount(0);
});

test("chat emoji picker preserves selection and drafts, uses text and fits every theme", async ({
  page,
}, info) => {
  const commands: Record<string, unknown>[] = [];
  const reads: unknown[] = [];
  await page.route("**/api/v1/rooms/read", (r) => {
    reads.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  let reject = false;
  await page.route("**/api/v1/commands", (r) => {
    const c = r.request().postDataJSON();
    commands.push(c);
    return reject && c.type === "send_message"
      ? r.fulfill({ status: 503, json: { error: "Connection interrupted" } })
      : r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await roomEntryFixture(page);
  await fixture(page, roomState());
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  const input = page.getByLabel("Message", { exact: true });
  await input.fill("Hello music");
  await input.evaluate((node: HTMLInputElement) => {
    node.focus();
    node.setSelectionRange(6, 11);
    node.dispatchEvent(new Event("select", { bubbles: true }));
  });
  await page.getByRole("button", { name: "Insert emoji", exact: true }).click();
  await page.getByLabel("Search emoji", { exact: true }).fill(":thumbsup:");
  await page
    .getByRole("button", { name: "thumbs up thumbsup 👍", exact: true })
    .click();
  await expect(input).toHaveValue("Hello 👍");
  expect(commands.filter((c) => c.type === "send_message")).toHaveLength(0);
  await page.getByRole("tab", { name: "Room board", exact: true }).click();
  await expect(input).not.toBeVisible();
  await pushRoomState(page, {
    ...roomState(),
    room_unread: { club: 1 },
    messages: [
      {
        id: 1,
        target: "club",
        username: "alice",
        text: "Unseen while viewing the board 🎵",
        timestamp: 1,
        private: false,
      },
    ],
  });
  await page.waitForTimeout(900);
  expect(reads).toHaveLength(0);
  await page.getByRole("tab", { name: "Chat", exact: true }).click();
  await expect(input).toHaveValue("Hello 👍");
  reject = true;
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Connection interrupted");
  await expect(input).toHaveValue("Hello 👍");
  reject = false;
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(input).toHaveValue("");
  expect(commands.filter((c) => c.type === "send_message").at(-1)).toEqual({
    type: "send_message",
    target: "club",
    text: "Hello 👍",
    private: false,
  });
  for (const theme of ["original", "light", "dark"]) {
    await page
      .getByRole("button", { name: "Open settings", exact: true })
      .click();
    await page.getByRole("tab", { name: "Appearance", exact: true }).click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page.getByRole("button", { name: "Rooms", exact: true }).click();
    await page
      .getByRole("button", { name: "Insert emoji", exact: true })
      .click();
    const box = await page
      .getByRole("dialog", { name: "Choose an emoji" })
      .boundingBox();
    const viewport = page.viewportSize()!;
    expect(
      box &&
        box.x >= 0 &&
        box.y >= 0 &&
        box.x + box.width <= viewport.width &&
        box.y + box.height <= viewport.height,
    ).toBe(true);
    await page.screenshot({
      path: `../artifacts/chat-emoji-${theme}-${info.project.name}.png`,
      fullPage: true,
    });
    await page.getByLabel("Search emoji", { exact: true }).press("Escape");
    await expect(
      page.getByRole("dialog", { name: "Choose an emoji" }),
    ).toHaveCount(0);
    await expect(input).toBeFocused();
  }
});

test("profile image chooser accepts ten MB, rejects larger files and displays processing", async ({
  page,
}) => {
  const empty = {
    description: "",
    picture: null,
    likes: [],
    dislikes: [],
    away_on_startup: false,
    away_message: "",
  };
  const posts: Record<string, unknown>[] = [];
  await page.route("**/api/v1/profile", async (r) => {
    if (r.request().method() === "GET") return r.fulfill({ json: empty });
    posts.push(r.request().postDataJSON());
    await new Promise((resolve) => setTimeout(resolve, 500));
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "Profile", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save profile", exact: true }),
  ).toBeEnabled();
  const input = page.getByLabel("Profile picture", { exact: true });
  const bytes = Buffer.alloc(10 * 1024 * 1024);
  Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=",
    "base64",
  ).copy(bytes);
  await input.setInputFiles({
    name: "too-large.png",
    mimeType: "image/png",
    buffer: Buffer.alloc(bytes.length + 1),
  });
  await expect(page.getByRole("alert")).toContainText("up to 10 MB");
  await expect(page.getByAltText("Your profile picture preview")).toHaveCount(
    0,
  );
  await input.setInputFiles({
    name: "accepted.png",
    mimeType: "image/png",
    buffer: bytes,
  });
  await expect(page.getByAltText("Your profile picture preview")).toBeVisible();
  await page.getByRole("button", { name: "Save profile", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Processing and saving…", exact: true }),
  ).toBeDisabled();
  await expect(input).toBeDisabled();
  await expect.poll(() => posts.length).toBe(1);
  expect(Buffer.from(posts[0].picture as string, "base64").length).toBe(
    bytes.length,
  );
});
test("owner access lists page past 2500 and require confirmation before removing a member", async ({
  page,
}, info) => {
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const requests = await roomEntryFixture(page, 3001);
  await fixture(page, roomState(roomSummary({ members: 3001 })));
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await page.getByRole("tab", { name: "Room board", exact: true }).click();
  await page.getByRole("tab", { name: "Room access", exact: true }).click();
  await page
    .getByLabel("Username to add to private room", { exact: true })
    .fill("carol");
  await page.getByRole("button", { name: "Add member", exact: true }).click();
  expect(commands).toContainEqual({
    type: "private_room_action",
    room: "club",
    action: "add_member",
    username: "carol",
  });
  for (let n = 0; n < 30; n++)
    await page
      .getByRole("button", { name: "Next member page", exact: true })
      .click();
  await expect(
    page.getByRole("button", { name: "Make operator user-3000", exact: true }),
  ).toBeVisible();
  expect(requests.some((q) => q.get("offset") === "3000")).toBe(true);
  await page
    .getByRole("button", { name: "Make operator user-3000", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "private_room_action",
    room: "club",
    action: "add_operator",
    username: "user-3000",
  });
  await page
    .getByRole("button", { name: "Remove member user-3000", exact: true })
    .click();
  expect(commands.filter((c) => c.action === "remove_member")).toHaveLength(0);
  await page
    .getByRole("button", { name: "Keep current access", exact: true })
    .click();
  expect(commands.filter((c) => c.action === "remove_member")).toHaveLength(0);
  await page
    .getByRole("button", { name: "Remove member user-3000", exact: true })
    .click();
  await page
    .getByRole("button", {
      name: "Confirm Remove member user-3000",
      exact: true,
    })
    .click();
  expect(commands).toContainEqual({
    type: "private_room_action",
    room: "club",
    action: "remove_member",
    username: "user-3000",
  });
  await page
    .getByRole("button", { name: "Relinquish room ownership", exact: true })
    .click();
  await expect(
    page.getByRole("group", {
      name: "Confirm room access change",
      exact: true,
    }),
  ).toContainText("does not transfer ownership");
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `../artifacts/private-room-access-${info.project.name}.png`,
    fullPage: true,
  });
});
test("operator controls respect role boundaries and cached roles cannot edit access", async ({
  page,
}) => {
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await roomEntryFixture(page);
  const state = roomState(roomSummary({ owner: "host", operator: true }));
  await fixture(page, state);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await page.getByRole("tab", { name: "Room board", exact: true }).click();
  await page.getByRole("tab", { name: "Room access", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Remove member bob", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Remove member alice", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Make operator/ })).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("button", {
      name: "Relinquish room ownership",
      exact: true,
    }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Resign as operator", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Confirm Resign as operator", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "private_room_action",
    room: "club",
    action: "resign_operator",
    username: null,
  });
  await page
    .getByRole("button", {
      name: "Cancel my private-room membership",
      exact: true,
    })
    .click();
  await page
    .getByRole("button", { name: "Confirm Cancel my membership", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "private_room_action",
    room: "club",
    action: "leave_membership",
    username: null,
  });
  await pushRoomState(page, {
    ...state,
    room_management: {
      ...state.room_management!,
      rooms: {
        club: roomSummary({
          owner: "host",
          operator: true,
          confirmed: false,
          owner_live: false,
        }),
      },
    },
  });
  await page
    .getByLabel("Username to add to private room", { exact: true })
    .fill("carol");
  await expect(
    page.getByRole("button", { name: "Add member", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByText("Cached — waiting for confirmation", { exact: true }),
  ).toBeVisible();
});

const profileFixture = (status = "online") => ({
  status,
  exists: true,
  pending: false,
  error: null,
  average_speed: 8192,
  shared_files: 3001,
  shared_folders: 1,
  description:
    "Music collector <script>window.untrustedProfileRan=true</script>",
  upload_slots: 4,
  queue_size: 2,
  slots_free: true,
  profile_pending: false,
  profile_error: null,
});

const discoveryFixture = (): NonNullable<Snapshot["discovery"]> => ({
  query: { mode: "personal" },
  recommendations: [
    { item: "blues", rating: 8 },
    { item: "<script>literal</script>", rating: 3 },
  ],
  against: [{ item: "muzak", rating: -2 }],
  people: [{ username: "alice", weight: 9 }],
  recommendations_at: 1700000000,
  people_at: 1700000000,
  recommendations_pending: false,
  people_pending: false,
  recommendations_error: null,
  people_error: null,
});
test("discovery links recommendations to interests, file searches and user profiles", async ({
  page,
}, info) => {
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    discovery: discoveryFixture(),
    users: {
      alice: {
        ...profileFixture(),
        interests: {
          likes: ["jazz"],
          dislikes: ["muzak"],
          updated_at: 1700000000,
        },
      },
    },
  };
  await fixture(page, state, true);
  await page.getByRole("button", { name: "Discover", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Discover", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Explore <script>literal</script>",
      exact: true,
    }),
  ).toBeVisible();
  expect(await page.locator(".discovery-list script").count()).toBe(0);
  await page.getByRole("button", { name: "Like blues", exact: true }).click();
  await expect
    .poll(() =>
      commands.some(
        (c) =>
          c.type === "set_interest" &&
          c.item === "blues" &&
          c.disposition === "like",
      ),
    )
    .toBe(true);
  await page
    .getByRole("button", { name: "Dislike blues", exact: true })
    .click();
  await expect
    .poll(() =>
      commands.some(
        (c) => c.type === "set_interest" && c.disposition === "dislike",
      ),
    )
    .toBe(true);
  await page
    .getByRole("button", { name: "User options for alice", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText("Interests");
  await expect(
    page.getByRole("button", { name: "Explore jazz from alice", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Explore jazz from alice", exact: true })
    .click();
  await expect
    .poll(() =>
      commands.some(
        (c) =>
          c.type === "discover" &&
          JSON.stringify(c.query) === '{"mode":"topic","item":"jazz"}',
      ),
    )
    .toBe(true);
  await page
    .getByLabel("Filter discovery results", { exact: true })
    .fill("blues");
  await expect(
    page.getByRole("button", {
      name: "Explore <script>literal</script>",
      exact: true,
    }),
  ).toHaveCount(0);
  await page.getByLabel("Filter discovery results", { exact: true }).fill("");
  await expect(
    page.getByText("Similarity score 9", { exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `../artifacts/discovery-${info.project.name}.png`,
    fullPage: true,
  });
  for (const theme of ["dark", "light"]) {
    await page
      .getByRole("button", { name: "Open settings", exact: true })
      .click();
    await page.getByRole("tab", { name: "Appearance", exact: true }).click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page.getByRole("button", { name: "Discover", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Explore blues", exact: true }),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `../artifacts/discovery-${theme}-${info.project.name}.png`,
      fullPage: true,
    });
  }
  await page
    .getByRole("button", { name: "Search files for blues", exact: true })
    .click();
  await expect
    .poll(() =>
      commands.some((c) => c.type === "search" && c.query === "blues"),
    )
    .toBe(true);
  await expect(
    page.getByRole("heading", {
      name: "Find your next favourite.",
      exact: true,
    }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Discover", exact: true }).click();
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Discover", exact: true }),
  ).toBeVisible();
});
test("discovery refresh retains cached data through partial replies, timeouts and disconnection", async ({
  page,
}) => {
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    discovery: discoveryFixture(),
  };
  await fixture(page, state);
  await page.getByRole("button", { name: "Discover", exact: true }).click();
  await page
    .getByRole("button", { name: "Refresh discovery", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "discover",
    query: { mode: "personal" },
    refresh: true,
  });
  const updating = {
    ...state,
    discovery: {
      ...state.discovery!,
      recommendations_pending: true,
      people_pending: true,
    },
  };
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    updating,
  );
  await expect(
    page.getByRole("button", { name: "Popular", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Explore blues", exact: true }),
  ).toBeVisible();
  const partial = {
    ...state,
    discovery: {
      ...state.discovery!,
      recommendations: [],
      recommendations_at: 1700000020,
      people_pending: true,
    },
  };
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    partial,
  );
  await expect(
    page.getByText(
      "The server returned no recommendations. Try another topic or add more interests.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByText("Finding people…", { exact: true }),
  ).toBeVisible();
  const failed = {
    ...state,
    discovery: {
      ...state.discovery!,
      people_error:
        "The Soulseek server did not answer in time. Try Refresh; any previous results are kept.",
    },
  };
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    failed,
  );
  await expect(page.getByRole("alert")).toContainText("did not answer in time");
  await expect(
    page.getByRole("button", { name: "Refresh discovery", exact: true }),
  ).toBeEnabled();
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    { ...failed, connection: "disconnected" },
  );
  await expect(
    page.getByRole("button", { name: "Refresh discovery", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Search files for blues", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Sign in to Soulseek", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "Accounts", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
});
test("popular and topic discovery send distinct requests and show the appropriate results", async ({
  page,
}) => {
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    discovery: discoveryFixture(),
  };
  await fixture(page, state);
  await page.getByRole("button", { name: "Discover", exact: true }).click();
  await page.getByRole("button", { name: "Popular", exact: true }).click();
  await expect
    .poll(() => commands)
    .toContainEqual({
      type: "discover",
      query: { mode: "popular" },
      refresh: false,
    });
  const popular = {
    ...state,
    discovery: {
      ...state.discovery!,
      query: { mode: "popular" as const },
      recommendations: [{ item: "rock", rating: 12 }],
      people: [],
    },
  };
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    popular,
  );
  await expect(
    page.getByRole("heading", { name: /Similar listeners/ }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Explore rock", exact: true }),
  ).toBeVisible();
  await page.getByLabel("Discovery topic", { exact: true }).fill("jazz");
  await page
    .getByRole("button", { name: "Explore topic", exact: true })
    .click();
  await expect
    .poll(() => commands)
    .toContainEqual({
      type: "discover",
      query: { mode: "topic", item: "jazz" },
      refresh: false,
    });
  const topic = {
    ...state,
    discovery: {
      ...state.discovery!,
      query: { mode: "topic" as const, item: "jazz" },
      recommendations: [{ item: "soul", rating: 5 }],
      people: [{ username: "bob", weight: null }],
    },
  };
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    topic,
  );
  await expect(
    page.getByRole("heading", { name: "Related to jazz", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: /People who like this/ }),
  ).toBeVisible();
  await expect(
    page.getByText("Similarity score 0", { exact: true }),
  ).toHaveCount(0);
  await page.getByText("Less recommended (1)", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Explore muzak", exact: true }),
  ).toBeVisible();
});

test("refresh restores a protected session and signing out revokes it", async ({
  page,
}) => {
  await fixture(page, emptySnapshot, true);
  await page.reload();
  await expect(page.getByRole("navigation")).toBeVisible();
  await expect(page.getByLabel("Username", { exact: true })).toHaveCount(0);
  const cookie = (await page.context().cookies()).find(
    (c) => c.name === "rustysoul_session",
  );
  expect(cookie?.httpOnly).toBe(true);
  expect(cookie?.sameSite).toBe("Strict");
  expect(await page.evaluate(() => document.cookie)).not.toContain(
    "rustysoul_session",
  );
  expect(
    await page.evaluate(() =>
      JSON.stringify({ ...localStorage, ...sessionStorage }),
    ),
  ).not.toContain("fixture-session");
  await page
    .getByRole("button", { name: "Sign out of workspace", exact: true })
    .click();
  await expect(page.getByLabel("Username", { exact: true })).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Sign in", exact: true }),
  ).toBeVisible();
});

test("infinite search scrolling passes 2500 files and retains selections and user ordering", async ({
  page,
}) => {
  test.setTimeout(90000);
  const total = 3100;
  const offsets: number[] = [];
  await page.route("**/api/v1/searches/paged/results?**", (r) => {
    const q = new URL(r.request().url()).searchParams;
    const offset = Number(q.get("offset"));
    offsets.push(offset);
    return r.fulfill({
      json: {
        total,
        offset,
        files: Array.from(
          { length: Math.min(100, total - offset) },
          (_, i) => ({
            username: "alice",
            path: `Music\\track-${String(offset + i).padStart(4, "0")}.flac`,
            size: 100,
            free_slots: 1,
            speed: 8192,
            attributes: {},
          }),
        ),
        users: { alice: { count: total, folders: 1, speed: 8192, slots: 1 } },
      },
    });
  });
  let batch: unknown;
  await page.route("**/api/v1/downloads/batch*", (r) => {
    batch = r.request().postDataJSON();
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, {
    ...emptySnapshot,
    connection: "connected",
    transfer_capacity: 256,
    searches: [
      {
        id: "paged",
        query: "music",
        collecting: false,
        total,
        revision: 1,
        truncated: false,
        results: [],
      },
    ],
  });
  await page
    .getByLabel("Select Music\\track-0000.flac from alice", { exact: true })
    .check();
  // Move through the automatic sentinel; no manual page number or pagination action.
  for (let i = 0; i < 35 && !offsets.includes(3000); i++) {
    const footer = page.locator(".infinite-sentinel").last();
    await footer.evaluate((el) => el.scrollIntoView({ block: "end" }));
    await page.waitForTimeout(300);
  }
  await expect.poll(() => offsets.includes(3000)).toBe(true);
  // The last folder rows are themselves revealed just in time.
  const last = page.getByLabel("Select Music\\track-3000.flac from alice", {
    exact: true,
  });
  for (let i = 0; i < 35 && (await last.count()) === 0; i++) {
    await page.locator(".infinite-sentinel").last().scrollIntoViewIfNeeded();
    await page.waitForTimeout(100);
  }
  await last.check();
  expect(await page.locator(".user-results tbody tr").count()).toBeLessThan(
    800,
  );
  await page
    .getByRole("button", { name: "Download selected", exact: true })
    .click();
  await expect
    .poll(() => batch)
    .toEqual({
      files: [
        { username: "alice", path: "Music\\track-0000.flac", size: 100 },
        { username: "alice", path: "Music\\track-3000.flac", size: 100 },
      ],
    });
  await expect(
    page.getByRole("button", { name: "Download selected", exact: true }),
  ).toBeDisabled();
  await page.getByLabel("Order users", { exact: true }).selectOption("speed");
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(page.getByLabel("Order users", { exact: true })).toHaveValue(
    "speed",
  );
});

test("uploads and downloads expose useful stable sort and reverse controls", async ({
  page,
}) => {
  const transfer = (path: string, size: number, speed: number) => ({
    username: "alice",
    path,
    size,
    transferred: 10,
    speed,
    status: speed ? "downloading" : "waiting",
    queue_position: null,
  });
  const downloads = [
    transfer("z.flac", 100, 0),
    transfer("a.flac", 100, 50),
    transfer("b.flac", 200, 10),
  ];
  const uploads = downloads.map((t) => ({
    ...t,
    status: t.speed ? "uploading" : "queued",
  }));
  await fixture(page, { ...emptySnapshot, downloads, uploads });
  for (const name of ["Downloads", "Uploads"]) {
    await page.getByRole("button", { name, exact: true }).click();
    await page
      .getByLabel("Order transfers", { exact: true })
      .selectOption("size");
    await expect(page.locator(".transfer-detail>strong").first()).toHaveText(
      "b.flac",
    );
    await page
      .getByLabel("Order transfers", { exact: true })
      .selectOption("name");
    await expect(page.locator(".transfer-detail>strong")).toHaveText([
      "a.flac",
      "b.flac",
      "z.flac",
    ]);
    await page
      .getByRole("button", { name: "Reverse transfer order", exact: true })
      .click();
    await expect(page.locator(".transfer-detail>strong")).toHaveText([
      "z.flac",
      "b.flac",
      "a.flac",
    ]);
  }
});

test("wishlist and sharing controls dispatch explicit persistent choices", async ({
  page,
}) => {
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ json: { status: "queued" } });
  });
  await fixture(page, {
    ...emptySnapshot,
    user_rules: [{ username: "alice", ignored: true, banned: false }],
  });
  await page.getByText("Wishlist · 0 enabled", { exact: true }).click();
  await page.getByLabel("Wishlist query", { exact: true }).fill("rare album");
  await page.getByRole("button", { name: "Add wish", exact: true }).click();
  expect(commands).toContainEqual({ type: "add_wish", query: "rare album" });
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "People & sharing", exact: true })
    .click();
  await expect(
    page.getByLabel("Ignore messages", { exact: true }),
  ).toBeChecked();
  await page.getByLabel("Ban sharing", { exact: true }).click();
  expect(commands).toContainEqual({
    type: "set_user_rule",
    username: "alice",
    ignored: true,
    banned: true,
  });
});

test("nested share folders expand and collapse independently", async ({
  page,
}, info) => {
  const state: Snapshot = {
    ...emptySnapshot,
    username: "alice",
    browse: {
      alice: {
        pending: false,
        total: 4000,
        files: [],
        truncated: false,
        revision: 1,
      },
    },
  };
  await page.route("**/api/v1/commands", (r) =>
    r.fulfill({ json: { status: "queued" } }),
  );
  await page.route("**/api/v1/browse/files?**", (r) =>
    r.fulfill({ json: { files: [], folders: [], total: 4000, offset: 0 } }),
  );
  await fixture(page, state);
  await page.route("**/api/v1/browse/tree?**", (r) => {
    const parent = new URL(r.request().url()).searchParams.get("parent");
    const names =
      parent === ""
        ? ["Music"]
        : parent === "Music"
          ? ["Artist A", "Artist B"]
          : parent === "Music\\Artist A"
            ? ["Album One", "Album Two"]
            : [];
    return r.fulfill({
      json: {
        folders: names.map((name) => ({
          name,
          path: parent ? `${parent}\\${name}` : name,
          files: 1000,
          direct_files: 0,
          has_children: parent === "" || parent === "Music",
        })),
        total: names.length,
        offset: 0,
      },
    });
  });
  await page.getByRole("button", { name: "Browse users", exact: true }).click();
  await page
    .getByRole("button", { name: "Browse my shares", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Expand folder Music", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Expand folder Music\\Artist A", exact: true })
    .click();
  await expect(
    page.getByTitle("Music\\Artist A\\Album One", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", {
      name: "Collapse folder Music\\Artist A",
      exact: true,
    })
    .click();
  await expect(
    page.getByTitle("Music\\Artist A\\Album One", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByTitle("Music\\Artist B", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Collapse all folders", exact: true })
    .click();
  await expect(page.getByTitle("Music\\Artist B", { exact: true })).toHaveCount(
    0,
  );
  await page.screenshot({
    path: `../artifacts/folder-tree-${info.project.name}.png`,
    fullPage: true,
  });
});

test("user actions show live status, profile and paged shares", async ({
  page,
}, info) => {
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    username: "my-self",
    users: { alice: profileFixture() },
    searches: [
      {
        id: "search",
        query: "album",
        collecting: false,
        results: [
          {
            username: "alice",
            path: "Music\\Album\\first.flac",
            size: 100,
            free_slots: 1,
            speed: 8192,
            attributes: {},
          },
        ],
      },
    ],
    browse: {
      alice: {
        pending: false,
        total: 3001,
        revision: 1,
        truncated: false,
        files: [],
      },
    },
  };
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ json: { status: "queued" } });
  });
  await page.route("**/api/v1/browse/files?**", (r) => {
    const q = new URL(r.request().url()).searchParams;
    return r.fulfill({
      json:
        q.get("folders") === "true"
          ? {
              folders: [{ path: "Music\\Album", files: 3001 }],
              files: [],
              total: 1,
              offset: 0,
            }
          : {
              folders: [],
              files: [
                {
                  username: "alice",
                  path: `Music\\Album\\track-${q.get("offset")}.flac`,
                  size: 100,
                  free_slots: 0,
                  speed: 0,
                  attributes: {},
                },
              ],
              total: 3001,
              offset: Number(q.get("offset")),
            },
    });
  });
  await fixture(page, state);
  await page.getByRole("button", { name: "User options for alice" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(
    page.getByRole("dialog").getByText("Online", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("dialog")).toContainText("3,001");
  await expect(page.getByRole("dialog")).toContainText(
    "<script>window.untrustedProfileRan=true</script>",
  );
  expect(
    await page.evaluate(() =>
      Boolean(
        (window as unknown as { untrustedProfileRan?: boolean })
          .untrustedProfileRan,
      ),
    ),
  ).toBe(false);
  await page.getByRole("button", { name: "Add buddy", exact: true }).click();
  expect(commands).toContainEqual({
    type: "set_buddy",
    username: "alice",
    buddy: true,
  });
  await page
    .getByRole("button", { name: "Browse shares", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Users & shared libraries" }),
  ).toBeVisible();
  await expect(
    page.getByText("3,001 shared files", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("track-0.flac", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Next", exact: true }).click();
  await expect(page.getByText("track-100.flac", { exact: true })).toBeVisible();
  await page.screenshot({
    path: `../artifacts/user-shares-${info.project.name}.png`,
    fullPage: true,
  });
  expect(commands).toContainEqual({ type: "lookup_user", username: "alice" });
  expect(commands).toContainEqual({ type: "browse", username: "alice" });
});

test("own cached library is browseable while disconnected", async ({
  page,
}) => {
  const state: Snapshot = {
    ...emptySnapshot,
    username: "my-self",
    users: { "my-self": profileFixture("offline") },
    browse: {
      "my-self": {
        pending: false,
        total: 94029,
        revision: 1,
        truncated: false,
        files: [],
      },
    },
  };
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ json: { status: "queued" } });
  });
  await page.route("**/api/v1/browse/files?**", (r) =>
    r.fulfill({ json: { files: [], folders: [], total: 0, offset: 0 } }),
  );
  await fixture(page, state);
  await page.getByRole("button", { name: "Browse users", exact: true }).click();
  await page
    .getByRole("button", { name: "Browse my shares", exact: true })
    .click();
  await expect(
    page.getByText("94,029 shared files", { exact: true }),
  ).toBeVisible();
  expect(commands).toContainEqual({ type: "browse", username: "my-self" });
  await page.getByRole("button", { name: "User options for my-self" }).click();
  await expect(
    page.getByRole("dialog").getByText("Offline", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Browse shares", exact: true }),
  ).toBeEnabled();
});

test("room members have clickable names and live presence", async ({
  page,
}, info) => {
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    joined_rooms: ["Music"],
    rooms: [{ name: "Music", users: 2 }],
    room_members: { Music: ["alice", "bob"] },
    users: { alice: profileFixture("away"), bob: profileFixture("offline") },
  };
  await page.route("**/api/v1/commands", (r) =>
    r.fulfill({ json: { status: "queued" } }),
  );
  await fixture(page, state);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await page.getByRole("button", { name: "Music 2 users" }).click();
  await page.getByText("Room members (2)", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "User options for alice" }),
  ).toBeVisible();
  await expect(page.getByText("Away", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "User options for alice" }).click();
  await expect(
    page.getByRole("dialog").getByText("Away", { exact: true }),
  ).toBeVisible();
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    { ...state, users: { ...state.users, alice: profileFixture("online") } },
  );
  await expect(
    page.getByRole("dialog").getByText("Online", { exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: `../artifacts/user-profile-${info.project.name}.png`,
    fullPage: true,
  });
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("startup defaults and library indexing preference are editable", async ({
  page,
}) => {
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ json: { status: "queued" } });
  });
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "Network settings" }).click();
  await expect(
    page.getByLabel("Connect to Soulseek when RustySoul starts"),
  ).toBeChecked();
  await page.getByRole("tab", { name: "File locations & library" }).click();
  await expect(
    page.getByLabel("Index the library when RustySoul starts"),
  ).not.toBeChecked();
  await page.getByLabel("Index the library when RustySoul starts").check();
  expect(commands).toContainEqual({
    type: "update_library_settings",
    settings: { index_on_startup: true },
  });
});

test("first login requires account replacement and the form is clickable", async ({
  page,
}) => {
  await page.route("**/api/v1/auth/login", (r) =>
    r.fulfill({
      json: {
        token: "setup",
        account: { username: "admin", must_change: true },
      },
    }),
  );
  await page.route("**/api/v1/auth/logout", (r) =>
    r.fulfill({ json: { status: "signed_out" } }),
  );
  await page.goto("/");
  await page.getByLabel("Username", { exact: true }).fill("admin");
  await page.getByLabel("Password", { exact: true }).fill("change-me-now");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Secure your administrator account" }),
  ).toBeVisible();
  await expect(page.getByRole("navigation")).toHaveCount(0);
  await page.getByLabel("New username").fill("newadmin");
  await page
    .getByLabel("New password", { exact: true })
    .fill("new-test-password");
  await page.getByLabel("Confirm new password").fill("new-test-password");
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Sign in", exact: true }),
  ).toBeVisible();
});

async function fixture(page: Page, state: Snapshot, remember = false) {
  await page.route("**/api/v1/auth/me", (route) => {
    const known =
      remember &&
      route
        .request()
        .headers()
        ["cookie"]?.includes("rustysoul_session=fixture");
    return route.fulfill(
      known
        ? { json: { username: "fixture-admin", must_change: false } }
        : { status: 401, json: { error: "Sign in" } },
    );
  });
  await page.route("**/api/v1/browse/tree?**", (route) =>
    route.fulfill({ json: { folders: [], total: 0, offset: 0 } }),
  );
  if (remember)
    await page.route("**/api/v1/auth/logout", (route) =>
      route.fulfill({
        json: { status: "signed_out" },
        headers: {
          "Set-Cookie":
            "rustysoul_session=; Path=/api/v1; Max-Age=0; HttpOnly; SameSite=Strict",
        },
      }),
    );
  await page.route("**/api/v1/status", (route) =>
    route.fulfill({ json: state }),
  );
  await page.addInitScript((snapshot) => {
    const original = window.fetch;
    window.fetch = (input, options) => {
      if (String(input).endsWith("/events")) {
        return Promise.resolve(
          new Response(
            new ReadableStream({
              start(controller) {
                (
                  window as unknown as {
                    pushFixtureState: (s: Snapshot) => void;
                  }
                ).pushFixtureState = (next) =>
                  controller.enqueue(
                    new TextEncoder().encode(
                      `event: state\ndata: ${JSON.stringify(next)}\n\n`,
                    ),
                  );
                controller.enqueue(
                  new TextEncoder().encode(
                    `event: state\ndata: ${JSON.stringify(snapshot)}\n\n`,
                  ),
                );
                options?.signal?.addEventListener("abort", () =>
                  controller.close(),
                );
              },
            }),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
        );
      }
      return original(input, options);
    };
  }, state);
  await page.route("**/api/v1/auth/login", (route) =>
    route.fulfill({
      ...(remember
        ? {
            headers: {
              "Set-Cookie":
                "rustysoul_session=fixture; Path=/api/v1; HttpOnly; SameSite=Strict; Max-Age=2592000",
            },
          }
        : {}),
      json: {
        token: "fixture-session",
        account: { username: "fixture-admin", must_change: false },
      },
    }),
  );
  await page.goto("/");
  await page.getByLabel("Username", { exact: true }).fill("fixture-admin");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation")).toBeVisible();
}

test("results regroup live by user and folder and respond to format filtering", async ({
  page,
}, info) => {
  const file = (username: string, path: string) => ({
    username,
    path,
    size: 100,
    free_slots: 1,
    speed: 10,
    attributes: {},
  });
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    searches: [
      {
        id: "live-groups",
        query: "live grouping",
        collecting: true,
        results: [
          file("alice", "Music/A/alice-one.flac"),
          file("alice", "Music/A/alice-two.mp3"),
          file("bob", "Music/B/bob-one.flac"),
        ],
      },
    ],
  };
  await fixture(page, state);
  await expect(page.locator(".user-results>summary .user-link")).toHaveText([
    "alice",
    "bob",
  ]);
  const next = {
    ...state,
    searches: [
      {
        ...state.searches[0],
        results: [
          ...state.searches[0].results,
          file("bob", "Music/B/Two/bob-two.mp3"),
          file("bob", "Music/B/Two/bob-three.mp3"),
        ],
      },
    ],
  };
  await page.evaluate(
    (snapshot) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(snapshot),
    next,
  );
  await expect(page.locator(".user-results>summary .user-link")).toHaveText([
    "bob",
    "alice",
  ]);
  await expect(page.locator(".folder-results>summary>span")).toHaveText([
    "Music/B",
    "Music/B/Two",
    "Music/A",
  ]);
  await page.screenshot({
    path: `../artifacts/grouped-search-${info.project.name}.png`,
    fullPage: true,
  });
  await page
    .getByLabel("Search format", { exact: true })
    .selectOption("lossless");
  await expect(page.locator(".user-results>summary .user-link")).toHaveText([
    "alice",
    "bob",
  ]);
  await expect(
    page.getByRole("button", { name: "Download bob-two.mp3", exact: true }),
  ).toHaveCount(0);
  await page.getByLabel("Search format", { exact: true }).selectOption("lossy");
  await expect(page.locator(".user-results>summary .user-link")).toHaveText([
    "bob",
    "alice",
  ]);
  await expect(
    page.getByRole("button", { name: "Download alice-one.flac", exact: true }),
  ).toHaveCount(0);
});

test("offline workspace has useful empty states and fits the viewport", async ({
  page,
}, info) => {
  await fixture(page, emptySnapshot);
  await expect(
    page.getByRole("heading", { name: "Find your next favourite." }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Search network" }),
  ).toBeDisabled();
  await expect(page.getByLabel("Search the Soulseek network")).toBeDisabled();
  await page
    .getByRole("button", {
      name: "Sign in to the Soulseek network",
      exact: true,
    })
    .click();
  const headings = await page.locator(".settings-grid h2").allTextContents();
  expect(headings.indexOf("Soulseek account")).toBeLessThan(
    headings.indexOf("Administrator account"),
  );
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "A little room for discovery" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Search", exact: true }).click();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBeTruthy();
  await page.screenshot({
    path: `../artifacts/workspace-${info.project.name}.png`,
    fullPage: true,
  });
});

test("search result queues the exact peer file and safely renders hostile names", async ({
  page,
}) => {
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    username: "fixture-user",
    searches: [
      {
        id: "test-search",
        query: "fixture music",
        collecting: false,
        results: [
          {
            username: "fixture-peer",
            path: "Music\\<img src=x onerror=alert(1)>.flac",
            size: 42000000,
            speed: 1000,
            free_slots: 1,
            attributes: { "0": 900 },
          },
        ],
      },
    ],
  };
  let command: unknown;
  await page.route("**/api/v1/downloads/batch*", async (route) => {
    command = route.request().postDataJSON();
    await route.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await expect(
    page.getByText("<img src=x onerror=alert(1)>.flac", { exact: true }),
  ).toBeVisible();
  expect(await page.locator("tbody img").count()).toBe(0);
  await page
    .getByRole("button", { name: "Download <img src=x onerror=alert(1)>.flac" })
    .click();
  await expect
    .poll(() => command)
    .toEqual({
      files: [
        {
          username: "fixture-peer",
          path: state.searches[0].results[0].path,
          size: 42000000,
        },
      ],
    });
  await page.getByLabel("Filter results").fill("nothing-matches");
  await expect(
    page.getByRole("heading", { name: "No matching files" }),
  ).toBeVisible();
});

test("room and message views remain usable without a network", async ({
  page,
}) => {
  await fixture(page, emptySnapshot);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await expect(page.getByLabel("Message", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Messages", exact: true }).click();
  await page.getByLabel("Message username").fill("some-user");
  await page.getByLabel("Message", { exact: true }).fill("Hello");
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeDisabled();
});

test("speed sorting and batch selection preserve exact files across filters", async ({
  page,
}, info) => {
  const f = (username: string, path: string, speed: number) => ({
    username,
    path,
    speed,
    size: 1024,
    free_slots: 1,
    attributes: {},
  });
  const results = [
    f("alice", "Music/a.flac", 1024),
    f("alice", "Music/b.mp3", 1024),
    f("bob", "Other/c.flac", 1048576),
  ];
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    searches: [{ id: "batch", query: "batch", collecting: true, results }],
  };
  let batch: unknown;
  await page.route("**/api/v1/downloads/batch*", async (r) => {
    batch = r.request().postDataJSON();
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await page.getByLabel("Order users", { exact: true }).selectOption("speed");
  await expect(page.locator(".user-results>summary .user-link")).toHaveText([
    "bob",
    "alice",
  ]);
  await expect(
    page.getByText("1 MiB/s reported", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Select all matching (3)", exact: true })
    .click();
  await page
    .getByLabel("Select Music/b.mp3 from alice", { exact: true })
    .uncheck();
  await page.getByLabel("Filter results", { exact: true }).fill("c.flac");
  await expect(
    page.getByText(/2 selected · 2 KiB \(1 hidden by filter\)/),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Download selected", exact: true })
    .click();
  await expect
    .poll(() => batch)
    .toEqual({
      files: [
        { username: "bob", path: "Other/c.flac", size: 1024 },
        { username: "alice", path: "Music/a.flac", size: 1024 },
      ],
    });
  await expect(
    page.getByRole("button", { name: "Deselect all", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Download selected", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await page.screenshot({
    path: `../artifacts/batch-results-${info.project.name}.png`,
    fullPage: true,
  });
});

test("folder browser adds multiple library paths and only saves on request", async ({
  page,
}, info) => {
  const state: Snapshot = {
    ...emptySnapshot,
    location_roots: ["/downloads", "/library"],
    locations: { downloads: "/downloads", shares: ["/library"] },
  };
  let command: unknown;
  await page.route("**/api/v1/commands", async (r) => {
    command = r.request().postDataJSON();
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await page.route("**/api/v1/folders*", (r) => {
    const path = new URL(r.request().url()).searchParams.get("path");
    const paths =
      path === null
        ? ["/library", "/downloads"]
        : path === "/library"
          ? ["/library/Jazz", "/library/Classical"]
          : [];
    return r.fulfill({
      json: {
        path,
        parent: path?.startsWith("/library/") ? "/library" : null,
        roots: state.location_roots,
        folders: paths.map((p) => ({
          name: p.split("/").at(-1) || p,
          path: p,
        })),
        truncated: false,
      },
    });
  });
  await fixture(page, state);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "File locations & library", exact: true })
    .click();
  for (const name of ["Jazz", "Classical"]) {
    await page
      .getByRole("button", {
        name: "Browse and add library folder",
        exact: true,
      })
      .click();
    const dialog = page.getByRole("dialog", {
      name: "Add a shared library folder",
    });
    await dialog
      .getByRole("button", { name: "▸ library", exact: true })
      .click();
    await dialog
      .getByRole("button", { name: `▸ ${name}`, exact: true })
      .click();
    await dialog
      .getByRole("button", { name: "Use this folder", exact: true })
      .click();
  }
  await expect(
    page.getByLabel("Shared library folders — one per line", { exact: true }),
  ).toHaveValue("/library/Jazz\n/library/Classical");
  expect(command).toBeUndefined();
  await page
    .getByRole("button", { name: "Save file locations", exact: true })
    .click();
  await expect
    .poll(() => command)
    .toEqual({
      type: "update_locations",
      downloads: "/downloads",
      shares: ["/library/Jazz", "/library/Classical"],
    });
  await page.screenshot({
    path: `../artifacts/connection-settings-${info.project.name}.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
});

test("connection settings and room order expose understandable controls", async ({
  page,
}) => {
  const state: Snapshot = {
    ...emptySnapshot,
    rooms: [
      { name: "Zulu", users: 100, last_activity: null },
      { name: "Alpha", users: 10, last_activity: 50 },
      { name: "Beta", users: 20, last_activity: 100 },
    ],
  };
  let command: unknown;
  await page.route("**/api/v1/commands", async (r) => {
    command = r.request().postDataJSON();
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await page
    .getByLabel("Order rooms", { exact: true })
    .selectOption("activity");
  await expect(page.locator(".room-item strong")).toHaveText([
    "Beta",
    "Alpha",
    "Zulu",
  ]);
  await page.getByLabel("Order rooms", { exact: true }).selectOption("name");
  await expect(page.locator(".room-item strong")).toHaveText([
    "Alpha",
    "Beta",
    "Zulu",
  ]);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "Network settings", exact: true })
    .click();
  await page
    .getByLabel("Incoming transfer port (TCP)", { exact: true })
    .fill("3456");
  await page
    .getByLabel("Automatically forward the transfer port with UPnP", {
      exact: true,
    })
    .check();
  await page
    .getByRole("button", { name: "Save connection settings", exact: true })
    .click();
  await expect
    .poll(() => command)
    .toEqual({
      type: "update_network",
      settings: { ...state.network, port: 3456, upnp: true },
    });
  await page
    .getByRole("button", { name: "Check router & connection", exact: true })
    .click();
  await expect.poll(() => command).toEqual({ type: "check_network" });
});

test("daemon errors and informational messages can be dismissed and new occurrences return", async ({
  page,
}) => {
  const state: Snapshot = {
    ...emptySnapshot,
    error: "Peer browse timed out; the user may be offline or unreachable",
    notice: "File locations saved",
  };
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", async (r) => {
    commands.push(r.request().postDataJSON());
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await page
    .getByRole("button", { name: "Dismiss error", exact: true })
    .click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Dismiss message", exact: true })
    .click();
  await expect(
    page.getByText("File locations saved", { exact: true }),
  ).toHaveCount(0);
  await expect.poll(() => commands.length).toBe(2);
  expect(commands).toEqual([
    { type: "dismiss_messages" },
    { type: "dismiss_messages" },
  ]);
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    { ...state, error: null, notice: null },
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    state,
  );
  await expect(page.getByRole("alert")).toContainText("Peer browse timed out");
});

test("library progress updates live and stays in settings when the site banner is disabled", async ({
  page,
}, info) => {
  const state: Snapshot = {
    ...emptySnapshot,
    library: {
      ...emptySnapshot.library,
      scanning: true,
      phase: "scanning",
      scan_id: 1,
      indexed: 94029,
      processed: 1000,
      folders: 50,
      bytes: 104857600,
      changed: 100,
      skipped: 900,
      elapsed_ms: 5000,
    },
  };
  await fixture(page, state);
  const banner = page.getByRole("status", {
    name: "Library update banner",
    exact: true,
  });
  await expect(banner).toContainText("1,000 files checked");
  const next = {
    ...state,
    library: {
      ...state.library,
      processed: 2500,
      folders: 100,
      changed: 200,
      skipped: 2300,
      elapsed_ms: 10000,
    },
  };
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    next,
  );
  await expect(banner).toContainText("2,500 files checked");
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "Appearance", exact: true }).click();
  await page
    .getByLabel("Show library update banner", { exact: true })
    .uncheck();
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await page
    .getByRole("tab", { name: "File locations & library", exact: true })
    .click();
  await expect(banner).toHaveCount(0);
  const details = page.getByRole("status", {
    name: "Library update details",
    exact: true,
  });
  await expect(details).toContainText("2,500 files checked");
  await page.screenshot({
    path: `../artifacts/library-progress-dark-${info.project.name}.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await page.evaluate(
    (s) =>
      (
        window as unknown as { pushFixtureState: (s: Snapshot) => void }
      ).pushFixtureState(s),
    {
      ...next,
      library: {
        ...next.library,
        phase: "idle",
        scanning: false,
        indexed: 2500,
      },
    },
  );
  await expect(details).toContainText("Library up to date");
  await expect(details).toContainText("2,500 files in the current index");
});

test("light dark original and system themes apply immediately and persist", async ({
  page,
}, info) => {
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "Appearance", exact: true }).click();
  const theme = page.getByLabel("Theme", { exact: true });
  for (const choice of ["dark", "light", "original"]) {
    await theme.selectOption(choice);
    await expect(page.locator("html")).toHaveAttribute("data-theme", choice);
    if (choice === "dark") {
      await expect(page.locator(".panel h2").first()).toHaveCSS(
        "color",
        "rgb(231, 237, 244)",
      );
      await expect(page.getByLabel("Theme", { exact: true })).toHaveCSS(
        "background-color",
        "rgb(38, 50, 62)",
      );
    }
  }
  await page.emulateMedia({ colorScheme: "dark" });
  await theme.selectOption("system");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.screenshot({
    path: `../artifacts/theme-light-${info.project.name}.png`,
    fullPage: true,
  });
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("rustysoul.preferences")!).theme,
    ),
  ).toBe("system");
});

test("network tab saves concurrency and shared user speed budgets and explains a fixed container port", async ({
  page,
}, info) => {
  const state = { ...emptySnapshot, port_binding: { fixed_port: 2234 } };
  let command: unknown;
  await page.route("**/api/v1/commands", async (r) => {
    command = r.request().postDataJSON();
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "Network settings", exact: true })
    .click();
  await expect(
    page.getByLabel("Incoming transfer port (TCP)", { exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByText(/This container publishes TCP 2234/),
  ).toBeVisible();
  await page
    .getByLabel("Maximum simultaneous uploads", { exact: true })
    .fill("3");
  await page
    .getByLabel("Maximum simultaneous downloads", { exact: true })
    .fill("5");
  await page
    .getByLabel("Total upload limit (KiB/s)", { exact: true })
    .fill("1000");
  await page
    .getByLabel("Upload limit for each user (KiB/s)", { exact: true })
    .fill("200");
  await page
    .getByText("Individual user speed overrides", { exact: true })
    .click();
  await page
    .getByRole("button", { name: "Add user override", exact: true })
    .click();
  await page.getByLabel("User 1 username", { exact: true }).fill("friend");
  await page.getByLabel("User 1 upload limit", { exact: true }).fill("400");
  await page
    .getByRole("button", { name: "Save connection settings", exact: true })
    .click();
  await expect
    .poll(() => command)
    .toEqual({
      type: "update_network",
      settings: {
        ...state.network,
        upload_slots: 3,
        download_slots: 5,
        upload_limit_kib: 1000,
        upload_per_user_limit_kib: 200,
        user_limits: [
          { username: "friend", upload_limit_kib: 400, download_limit_kib: 0 },
        ],
      },
    });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await page.screenshot({
    path: `../artifacts/network-tab-${info.project.name}.png`,
    fullPage: true,
  });
});

test("clear transfer history filters finished uploads and downloads and retains active work", async ({
  page,
}) => {
  const record = (path: string, status: string) => ({
    username: "peer",
    path,
    status,
    size: 100,
    transferred: status === "completed" ? 100 : 0,
    speed: 0,
    queue_position: null,
  });
  const state = {
    ...emptySnapshot,
    connection: "connected",
    downloads: [
      record("a.flac", "completed"),
      record("b.flac", "failed"),
      record("c.flac", "downloading"),
    ],
    uploads: [record("d.flac", "cancelled"), record("e.flac", "uploading")],
  };
  let command: unknown;
  await page.route("**/api/v1/commands", async (r) => {
    command = r.request().postDataJSON();
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await page
    .getByLabel("Clear transfer history status", { exact: true })
    .selectOption("failed");
  await page
    .getByRole("button", { name: "Clear 1 history records", exact: true })
    .click();
  await expect
    .poll(() => command)
    .toEqual({
      type: "clear_transfers",
      direction: "downloads",
      status: "failed",
    });
  await expect(page.getByText("c.flac", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Uploads", exact: true }).click();
  await page
    .getByLabel("Clear transfer history status", { exact: true })
    .selectOption("all");
  await page
    .getByRole("button", { name: "Clear 1 history records", exact: true })
    .click();
  await expect
    .poll(() => command)
    .toEqual({ type: "clear_transfers", direction: "uploads", status: "all" });
  await expect(page.getByText("e.flac", { exact: true })).toBeVisible();
});

test("upload groups collapse independently and historic bandwidth ranges remain accessible", async ({
  page,
}) => {
  const since = Math.floor(Date.now() / 1000) - 200 * 86400;
  const traffic = { since, downloaded: 1048576, uploaded: 2097152 };
  const record = (username: string, path: string) => ({
    username,
    path,
    status: "completed",
    size: 100,
    transferred: 100,
    speed: 0,
    queue_position: null,
  });
  const uploads = [
    record("alice", "one.flac"),
    record("alice", "two.flac"),
    record("bob", "three.flac"),
  ];
  await page.route("**/api/v1/traffic?**", (r) => {
    const q = new URL(r.request().url()).searchParams;
    return r.fulfill({
      json: {
        totals: traffic,
        from: Number(q.get("from")),
        to: Number(q.get("to")),
        interval: 86400,
        points: [
          {
            start: Math.floor(since / 86400) * 86400,
            downloaded: 1048576,
            uploaded: 2097152,
          },
        ],
      },
    });
  });
  await fixture(page, { ...emptySnapshot, uploads, traffic });
  await page.getByRole("button", { name: "Uploads", exact: true }).click();
  await expect(
    page.getByLabel("Upload and download volume over time"),
  ).toBeVisible();
  await expect(
    page.getByText("All-time uploaded", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("2.00", { exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "Collapse uploads for alice", exact: true })
    .click();
  await expect(page.locator(".transfer-detail>strong")).toHaveText([
    "three.flac",
  ]);
  await page
    .getByRole("button", { name: "Expand uploads for alice", exact: true })
    .click();
  await expect(page.locator(".transfer-detail>strong")).toHaveCount(3);
  await page
    .getByRole("button", { name: "Collapse all users", exact: true })
    .click();
  await expect(page.locator(".transfer-detail>strong")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Expand all users", exact: true })
    .click();
  await expect(page.locator(".transfer-detail>strong")).toHaveCount(3);
  const allRequest = page.waitForRequest(
    (r) =>
      r.url().includes("/traffic?") &&
      new URL(r.url()).searchParams.get("from") === String(since),
  );
  await page.getByLabel("Bandwidth graph range").selectOption("all");
  await allRequest;
  await page.getByLabel("Bandwidth graph range").selectOption("custom");
  await expect(page.getByLabel("Graph start date")).toBeVisible();
  await expect(page.getByLabel("Inspect graph time")).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("transfer history loads just in time beyond 2500 records and resets on sorting", async ({
  page,
}) => {
  test.setTimeout(60000);
  const requests: URLSearchParams[] = [];
  await page.route("**/api/v1/transfers/downloads?**", (r) => {
    const q = new URL(r.request().url()).searchParams;
    requests.push(q);
    const offset = Number(q.get("offset")),
      count = Number(q.get("limit"));
    return r.fulfill({
      json: {
        transfers: Array.from(
          { length: Math.min(count, 4000 - offset) },
          (_, i) => ({
            username: "alice",
            path: `track-${offset + i}.flac`,
            size: 100,
            transferred: 100,
            speed: 0,
            status: "completed",
            queue_position: null,
          }),
        ),
        total: 4000,
        offset,
        next_offset: offset + count,
        summary: {
          total: 4000,
          active: 0,
          speed: 0,
          clearable: { all: 4000, completed: 4000 },
        },
      },
    });
  });
  await fixture(page, {
    ...emptySnapshot,
    downloads_total: 4000,
    downloads: [],
  });
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await expect(
    page.getByRole("button", {
      name: "Clear 4000 history records",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByLabel("Transfer page", { exact: true })).toHaveCount(
    0,
  );
  for (let i = 0; i < 28; i++) {
    await page
      .getByRole("button", { name: "Load more transfers", exact: true })
      .scrollIntoViewIfNeeded();
    await expect
      .poll(() => Math.max(...requests.map((q) => Number(q.get("offset")))))
      .toBeGreaterThanOrEqual((i + 1) * 100);
  }
  // Blocks far from the viewport unmount their rows without discarding data.
  expect(await page.locator(".transfer-row").count()).toBeLessThan(500);
  await page
    .getByLabel("Order transfers", { exact: true })
    .selectOption("speed");
  await expect
    .poll(() =>
      requests.some(
        (q) => q.get("order") === "speed" && q.get("offset") === "0",
      ),
    )
    .toBe(true);
});

test("profile settings save supported public fields and format precedes wishlist", async ({
  page,
}) => {
  let profile = {
    description: "My collection <script>literal</script>",
    picture: null,
    likes: ["jazz"],
    dislikes: [],
    away_on_startup: false,
    away_message: "",
  };
  const posts: object[] = [];
  await page.route("**/api/v1/profile", async (r) => {
    if (r.request().method() === "POST") {
      posts.push(r.request().postDataJSON());
      profile = r.request().postDataJSON();
      await r.fulfill({ status: 202, json: { status: "queued" } });
    } else await r.fulfill({ json: profile });
  });
  await fixture(page, { ...emptySnapshot, username: "my-user" });
  const format = await page.getByLabel("Search format").boundingBox();
  const wish = await page.locator(".wishlist-panel summary").boundingBox();
  expect(format && wish && format.y < wish.y).toBe(true);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "Profile", exact: true }).click();
  await expect(page.getByLabel("Profile description")).toHaveValue(
    profile.description,
  );
  await page.getByLabel("Profile description").fill("Updated about me");
  await page.getByLabel("Things I like").fill("ambient\njazz");
  await page.getByLabel("Automatic Away reply").fill("Back soon");
  await page
    .getByRole("checkbox", { name: "Start in Away mode", exact: true })
    .check();
  await page.getByRole("button", { name: "Save profile", exact: true }).click();
  await expect.poll(() => posts.length).toBe(1);
  expect(posts[0]).toMatchObject({
    description: "Updated about me",
    likes: ["ambient", "jazz"],
    away_on_startup: true,
    away_message: "Back soon",
  });
  await page.screenshot({
    path: `../artifacts/profile-${test.info().project.name}.png`,
    fullPage: true,
  });
  await page
    .getByRole("tab", { name: "Network settings", exact: true })
    .click();
  await expect(
    page.getByRole("checkbox", {
      name: "Automatically retry failed or timed-out downloads",
      exact: true,
    }),
  ).toBeChecked();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("per-user transfer totals and date or amount ordering are available without corrupt characters", async ({
  page,
}) => {
  const requests: string[] = [];
  const since = Math.floor(Date.now() / 1000) - 100;
  await page.route("**/api/v1/transfers/uploads?**", (r) => {
    requests.push(r.request().url());
    const q = new URL(r.request().url()).searchParams;
    const names =
      q.get("user_sort") === "most" ? ["bob", "alice"] : ["alice", "bob"];
    return r.fulfill({
      json: {
        transfers: names.map((username) => ({
          username,
          path: username + ".flac",
          size: 100,
          transferred: 100,
          speed: 0,
          status: "completed",
          queue_position: null,
        })),
        total: 2,
        summary: { total: 2, active: 0, speed: 0, clearable: { all: 2 } },
        users_since: since,
        users: {
          alice: {
            uploaded: 1024,
            downloaded: 2048,
            first_at: since,
            last_at: since,
            files: 1,
          },
          bob: {
            uploaded: 4096,
            downloaded: 1024,
            first_at: since,
            last_at: since,
            files: 1,
          },
        },
      },
    });
  });
  await fixture(page, {
    ...emptySnapshot,
    uploads_total: 2,
    transfer_revision: 1,
  });
  await page.getByRole("button", { name: "Uploads", exact: true }).click();
  await expect(page.locator(".user-transfer-totals")).toHaveCount(2);
  await page
    .getByLabel("Order transfer users", { exact: true })
    .selectOption("most");
  await expect(
    page
      .locator(".upload-group-heading")
      .first()
      .getByRole("button", { name: "User options for bob", exact: true }),
  ).toBeVisible();
  for (const order of ["least", "newest", "oldest", "recent"]) {
    await page
      .getByLabel("Order transfer users", { exact: true })
      .selectOption(order);
    await expect
      .poll(() =>
        requests.some(
          (url) => new URL(url).searchParams.get("user_sort") === order,
        ),
      )
      .toBe(true);
  }
  expect(await page.locator("main").innerText()).not.toContain("\uFFFD");
});

test("own profile picture displays and completed downloads open an offline audio player", async ({
  page,
}, testInfo) => {
  await page.route("**/api/v1/commands", (r) =>
    r.fulfill({ status: 202, json: { status: "queued" } }),
  );
  await page.route("**/api/v1/browse/files?**", (r) =>
    r.fulfill({ json: { files: [], folders: [], total: 0, offset: 0 } }),
  );
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF9sAAAAASUVORK5CYII=",
    "base64",
  );
  await page.route("**/api/v1/users/picture?**", (r) =>
    r.fulfill({ contentType: "image/png", body: png }),
  );
  await page.route("**/api/v1/downloads/file?**", (r) =>
    r.fulfill({ status: 404, json: { error: "Moved file" } }),
  );
  const info = {
    ...profileFixture("offline"),
    has_picture: true,
    picture_revision: "photo",
  };
  await fixture(page, {
    ...emptySnapshot,
    username: "myself",
    users: { myself: info },
    browse: {
      myself: {
        pending: false,
        total: 0,
        revision: 1,
        truncated: false,
        files: [],
      },
    },
    downloads: [
      {
        username: "peer",
        path: "Music\\song.flac",
        local_path: "peer/Music/song.flac",
        size: 10,
        transferred: 10,
        speed: 0,
        status: "completed",
        queue_position: null,
      },
    ],
  });
  await page.getByRole("button", { name: "Browse users", exact: true }).click();
  await page
    .getByRole("button", { name: "Browse my shares", exact: true })
    .click();
  await page
    .getByRole("button", { name: "User options for myself", exact: true })
    .click();
  const picture = page.getByRole("img", { name: "myself's profile picture" });
  await expect(picture).toBeVisible();
  await expect
    .poll(() => picture.evaluate((el: HTMLImageElement) => el.naturalWidth))
    .toBe(1);
  await page.screenshot({
    path: `../artifacts/picture-view-${testInfo.project.name}.png`,
    fullPage: true,
  });
  await page
    .getByRole("button", { name: "Close user details", exact: true })
    .click();
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await page
    .getByRole("button", { name: "Play song.flac", exact: true })
    .click();
  await expect(page.getByLabel("Audio playback")).toBeVisible();
  await expect(page.getByRole("alert")).toContainText(
    "missing from its recorded location",
  );
  await expect(
    page.getByRole("link", { name: "Save file", exact: true }),
  ).toHaveAttribute("href", /download=true/);
});

test("bandwidth graph series can be selected independently and sharing protection is configurable", async ({
  page,
}, testInfo) => {
  const now = Math.floor(Date.now() / 1000);
  await page.route("**/api/v1/traffic?**", (r) =>
    r.fulfill({
      json: {
        totals: { downloaded: 100, uploaded: 200, since: now - 3600 },
        from: now - 3600,
        to: now,
        interval: 60,
        points: [],
      },
    }),
  );
  await page.route("**/api/v1/transfers/uploads?**", (r) =>
    r.fulfill({
      json: {
        transfers: [],
        total: 0,
        summary: { total: 0, active: 0, speed: 0, clearable: {} },
      },
    }),
  );
  const commands: object[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, {
    ...emptySnapshot,
    traffic: { downloaded: 100, uploaded: 200, since: now - 3600 },
  });
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await expect(
    page.getByRole("checkbox", { name: "Downloads", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: "Uploads", exact: true }),
  ).toBeChecked();
  await page.getByRole("checkbox", { name: "Uploads", exact: true }).uncheck();
  await expect(page.locator(".traffic-chart path.upload-series")).toHaveCount(
    0,
  );
  await expect(page.locator(".traffic-chart path.download-series")).toHaveCount(
    1,
  );
  await page
    .getByRole("checkbox", { name: "Downloads", exact: true })
    .uncheck();
  await expect(
    page.getByText("Select uploads or downloads to show a graph."),
  ).toBeVisible();
  await page.getByRole("checkbox", { name: "Uploads", exact: true }).check();
  await expect(page.locator(".traffic-chart path.upload-series")).toHaveCount(
    1,
  );
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "People & sharing", exact: true })
    .click();
  await page.screenshot({
    path: `../artifacts/sharing-protection-${testInfo.project.name}.png`,
    fullPage: true,
  });
  await expect(page.getByLabel("Upload sharing policy")).toHaveValue("sharing");
  await page.getByLabel("Minimum shared files", { exact: true }).fill("20");
  await page.getByLabel("Sharing protection exceptions").fill("trusted-user");
  await page
    .getByRole("button", { name: "Save sharing protection", exact: true })
    .click();
  await expect.poll(() => commands.length).toBe(1);
  expect(commands[0]).toMatchObject({
    type: "update_sharing_policy",
    settings: {
      mode: "sharing",
      min_files: 20,
      exceptions: ["trusted-user"],
      allow_unknown: false,
    },
  });
});

test("room tabs keep separate drafts and unread indicators clear only after the latest messages are visible", async ({
  page,
}, testInfo) => {
  let state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    username: "myself",
    joined_rooms: ["Room A", "Room B"],
    subscribed_rooms: ["Room A", "Room B"],
    room_read: { "Room A": 0, "Room B": 0 },
    room_unread: { "Room A": 1, "Room B": 1 },
    rooms: [
      { name: "Room A", users: 5 },
      { name: "Room B", users: 4 },
    ],
    messages: [
      {
        id: 1,
        target: "Room A",
        username: "peer",
        text: "First room message",
        private: false,
        timestamp: 1,
      },
      {
        id: 2,
        target: "Room B",
        username: "peer",
        text: "Second room message",
        private: false,
        timestamp: 2,
      },
    ],
  };
  const acknowledgements: { room: string; through_id: number }[] = [];
  await page.route("**/api/v1/rooms/read", async (r) => {
    const read = r.request().postDataJSON();
    acknowledgements.push(read);
    state = {
      ...state,
      room_read: { ...state.room_read, [read.room]: read.through_id },
      room_unread: { ...state.room_unread, [read.room]: 0 },
    };
    await r.fulfill({ status: 202, json: { status: "queued" } });
    await page.evaluate(
      (next) => (window as any).pushFixtureState(next),
      state,
    );
  });
  await fixture(page, state);
  await expect(
    page.getByLabel("2 unread room messages", { exact: true }),
  ).toBeVisible();
  expect(acknowledgements).toHaveLength(0);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await expect(
    page.getByRole("tab", { name: "Room A", exact: true }),
  ).toBeVisible();
  await page.locator(".message-list").scrollIntoViewIfNeeded();
  await expect
    .poll(() =>
      acknowledgements.some((a) => a.room === "Room A" && a.through_id === 1),
    )
    .toBe(true);
  await expect(
    page.getByLabel("1 unread room messages", { exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: `../artifacts/room-tabs-${testInfo.project.name}.png`,
    fullPage: true,
  });
  await page.getByLabel("Message", { exact: true }).fill("Draft A");
  await page.getByRole("tab", { name: "Room B", exact: true }).click();
  await expect(page.getByLabel("Message", { exact: true })).toHaveValue("");
  await page.getByLabel("Message", { exact: true }).fill("Draft B");
  await page.getByRole("tab", { name: "Room A", exact: true }).click();
  await expect(page.getByLabel("Message", { exact: true })).toHaveValue(
    "Draft A",
  );
  await page.getByRole("button", { name: "Search", exact: true }).click();
  state = {
    ...state,
    room_unread: { "Room A": 0, "Room B": 2 },
    messages: [
      ...state.messages,
      {
        id: 3,
        target: "Room B",
        username: "peer",
        text: "Another unseen message",
        private: false,
        timestamp: 3,
      },
    ],
  };
  await page.evaluate((next) => (window as any).pushFixtureState(next), state);
  await expect(
    page.getByLabel("2 unread room messages", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await page.getByRole("tab", { name: "Room B", exact: true }).click();
  await page.locator(".message-list").scrollIntoViewIfNeeded();
  await expect
    .poll(() =>
      acknowledgements.some((a) => a.room === "Room B" && a.through_id === 3),
    )
    .toBe(true);
  await expect(
    page.getByLabel("2 unread room messages", { exact: true }),
  ).toHaveCount(0);
});

test("long unread room histories stay unread until the latest message is visible", async ({
  page,
}) => {
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    username: "myself",
    joined_rooms: ["Long room"],
    subscribed_rooms: ["Long room"],
    room_read: { "Long room": 0 },
    room_unread: { "Long room": 80 },
    rooms: [{ name: "Long room", users: 5 }],
    messages: Array.from({ length: 80 }, (_, i) => ({
      id: i + 1,
      target: "Long room",
      username: "peer",
      text: `Unseen message ${i + 1}`,
      private: false,
      timestamp: i + 1,
    })),
  };
  const reads: unknown[] = [];
  await page.route("**/api/v1/rooms/read", async (r) => {
    reads.push(r.request().postDataJSON());
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  const history = page.locator(".message-list");
  await history.scrollIntoViewIfNeeded();
  await expect(
    page.getByText("Unseen message 1", { exact: true }),
  ).toBeInViewport();
  await expect(
    page.getByText("Unseen message 80", { exact: true }),
  ).not.toBeInViewport();
  // Stay in the first unread portion for longer than the read acknowledgement interval.
  await page.waitForTimeout(1700);
  expect(reads).toHaveLength(0);
  await page
    .getByRole("button", { name: "New messages · Jump to latest", exact: true })
    .click();
  await expect
    .poll(() =>
      reads.some((r: any) => r.room === "Long room" && r.through_id === 80),
    )
    .toBe(true);
});

test("completed audio plays in the browser while Soulseek is offline", async ({
  page,
}, testInfo) => {
  const samples = 16000;
  const wav = Buffer.alloc(44 + samples * 2);
  wav.write("RIFF");
  wav.writeUInt32LE(wav.length - 8, 4);
  wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(8000, 24);
  wav.writeUInt32LE(16000, 28);
  wav.writeUInt16LE(2, 32);
  wav.writeUInt16LE(16, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(samples * 2, 40);
  for (let i = 0; i < samples; i++)
    wav.writeInt16LE(
      Math.round(Math.sin((i * 2 * Math.PI * 440) / 8000) * 500),
      44 + i * 2,
    );
  await page.route("**/api/v1/downloads/file?**", (r) =>
    r.fulfill({ status: 200, contentType: "audio/wav", body: wav }),
  );
  await fixture(page, {
    ...emptySnapshot,
    downloads: [
      {
        username: "peer",
        path: "Music\\Album\\song.wav",
        local_path: "peer/Music/Album/song.wav",
        size: wav.length,
        transferred: wav.length,
        speed: 0,
        status: "completed",
        queue_position: null,
      },
    ],
  });
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await page
    .getByRole("button", { name: "Play song.wav", exact: true })
    .click();
  await expect(
    page.getByText("Saved to peer/Music/Album/song.wav", { exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator("audio")
        .evaluate((el: HTMLAudioElement) => !el.paused && el.currentTime > 0),
    )
    .toBe(true);
  await page.screenshot({
    path: `../artifacts/audio-player-${testInfo.project.name}.png`,
    fullPage: true,
  });
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(page.getByLabel("Audio playback")).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator("audio")
        .evaluate((el: HTMLAudioElement) => el.currentTime > 0.1),
    )
    .toBe(true);
});

test("refresh restores the current section and Settings logs filter safely", async ({
  page,
}, info) => {
  const queries: string[] = [];
  await page.route("**/api/v1/logs?**", (r) => {
    queries.push(r.request().url());
    return r.fulfill({
      json: {
        entries: [
          {
            id: 5,
            at: 1794830000,
            level: "error",
            source: "system",
            message:
              "Cannot open staging folder: Permission denied <script>literal</script>",
          },
        ],
        more: false,
      },
    });
  });
  await fixture(page, emptySnapshot, true);
  await page.getByRole("button", { name: "Uploads", exact: true }).click();
  await page.reload();
  await expect(
    page.getByText("Upload activity", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "Logs & activity", exact: true }).click();
  await expect(
    page.getByRole("log", { name: "Operational history" }),
  ).toContainText("Permission denied <script>literal</script>");
  expect(await page.locator(".operational-log script").count()).toBe(0);
  await page.getByLabel("Log level", { exact: true }).selectOption("error");
  await page.getByLabel("Filter logs", { exact: true }).fill("staging");
  await expect
    .poll(() =>
      queries.some(
        (u) => u.includes("level=error") && u.includes("filter=staging"),
      ),
    )
    .toBe(true);
  await page
    .getByRole("button", { name: "Pause live logs", exact: true })
    .click();
  await page.screenshot({
    path: `../artifacts/operational-logs-${info.project.name}.png`,
    fullPage: true,
  });
  await page.reload();
  await expect(
    page.getByRole("tab", { name: "Logs & activity", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
});

test("failed downloads can be retried and a user's uploads can be cancelled in bulk", async ({
  page,
}) => {
  const commands: any[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const transfer = (username: string, path: string, status: string) => ({
    username,
    path,
    status,
    size: 100,
    transferred: 0,
    speed: 0,
    queue_position: null,
  });
  await fixture(page, {
    ...emptySnapshot,
    connection: "connected",
    downloads: [transfer("alice", "failed.flac", "failed")],
    uploads: [
      transfer("alice", "a.flac", "queued"),
      transfer("alice", "b.flac", "uploading"),
      transfer("bob", "other.flac", "queued"),
    ],
  });
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await page
    .getByRole("button", { name: "Retry failed downloads", exact: true })
    .click();
  expect(commands).toContainEqual({ type: "retry_downloads", username: null });
  await page
    .getByRole("button", { name: "Retry failed.flac", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "transfer",
    username: "alice",
    path: "failed.flac",
    action: "resume",
  });
  await page.getByRole("button", { name: "Uploads", exact: true }).click();
  await page
    .getByLabel("Select upload a.flac for alice", { exact: true })
    .check();
  await page
    .getByRole("button", {
      name: "Cancel selected uploads for alice (1)",
      exact: true,
    })
    .click();
  expect(commands).toContainEqual({
    type: "cancel_uploads",
    username: "alice",
    paths: ["a.flac"],
  });
  await page
    .getByRole("button", { name: "Cancel all uploads for alice", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "cancel_uploads",
    username: "alice",
    paths: [],
  });
  expect(
    commands.some((c) => c.type === "cancel_uploads" && c.username === "bob"),
  ).toBe(false);
});

test("completed searches repeat edited terms and user sort survives navigation and refresh", async ({
  page,
}) => {
  const commands: any[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(
    page,
    {
      ...emptySnapshot,
      connection: "connected",
      searches: [
        {
          id: "repeat-search",
          query: "old album",
          collecting: false,
          results: [
            {
              username: "alice",
              path: "Music/track.flac",
              size: 1024,
              speed: 10,
              free_slots: 1,
              attributes: {},
            },
          ],
        },
      ],
    },
    true,
  );
  await page
    .getByLabel("Repeat search terms", { exact: true })
    .fill("new album");
  await page.getByRole("button", { name: "Search again", exact: true }).click();
  expect(commands).toContainEqual({
    type: "restart_search",
    id: "repeat-search",
    query: "new album",
  });
  await page
    .getByLabel("Order users", { exact: true })
    .selectOption("available");
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(page.getByLabel("Order users", { exact: true })).toHaveValue(
    "available",
  );
  await page.reload();
  await expect(page.getByLabel("Order users", { exact: true })).toHaveValue(
    "available",
  );
});

test("share selections persist across filters and folders, include sizes, and clear only on queue admission", async ({
  page,
}, info) => {
  const a = {
    username: "alice",
    path: "Music\\Album A\\one.flac",
    size: 1024,
    speed: 0,
    free_slots: 0,
    attributes: {},
  };
  const b = { ...a, path: "Music\\Album B\\two.flac", size: 2048 };
  await page.route("**/api/v1/commands", (r) =>
    r.fulfill({ status: 202, json: { status: "queued" } }),
  );
  await page.route("**/api/v1/browse/files?**", (r) => {
    const q = new URL(r.request().url()).searchParams;
    const folder = q.get("folder"),
      filter = q.get("filter");
    let files = [a, b].filter(
      (f) =>
        (!folder || f.path.startsWith(folder + "\\")) &&
        (!filter || f.path.includes(filter)),
    );
    return r.fulfill({
      json: {
        files: files.map((f) => ({
          ...f,
          username: q.get("username") || "alice",
        })),
        folders: [],
        offset: 0,
        total: files.length,
      },
    });
  });
  let reject = true;
  const batches: any[] = [];
  await page.route("**/api/v1/downloads/batch*", (r) => {
    batches.push(r.request().postDataJSON());
    return r.fulfill({
      status: reject ? 409 : 202,
      json: reject ? { error: "Download queue is full" } : { status: "queued" },
    });
  });
  await fixture(page, {
    ...emptySnapshot,
    connection: "connected",
    username: "myself",
    transfer_capacity: 256,
    browse: {
      alice: {
        pending: false,
        files: [],
        total: 2,
        truncated: false,
        revision: 1,
      },
      myself: {
        pending: false,
        files: [],
        total: 2,
        truncated: false,
        revision: 1,
      },
    },
  });
  await page.route("**/api/v1/browse/tree?**", (r) => {
    const parent = new URL(r.request().url()).searchParams.get("parent");
    const folders =
      parent === ""
        ? [
            {
              path: "Music",
              name: "Music",
              files: 2,
              direct_files: 0,
              has_children: true,
              subfolders: 2,
              direct_folders: 2,
            },
          ]
        : parent === "Music"
          ? [
              {
                path: "Music\\Album A",
                name: "Album A",
                files: 1,
                direct_files: 1,
                has_children: false,
                subfolders: 0,
                direct_folders: 0,
              },
              {
                path: "Music\\Album B",
                name: "Album B",
                files: 1,
                direct_files: 1,
                has_children: false,
                subfolders: 0,
                direct_folders: 0,
              },
            ]
          : [];
    return r.fulfill({ json: { folders, total: folders.length, offset: 0 } });
  });
  await page.getByRole("button", { name: "Browse users", exact: true }).click();
  await page.getByRole("button", { name: /^alice Unknown$/ }).click();
  await page
    .getByRole("button", { name: "Expand all folders", exact: true })
    .click();
  await expect(
    page.getByTitle("Music\\Album A", { exact: true }),
  ).toBeVisible();
  await expect(page.getByTitle("Music", { exact: true })).toContainText(
    "2 subfolders",
  );
  await page
    .getByLabel("Select shared folder Music\\Album A", { exact: true })
    .check();
  await expect(
    page.getByRole("button", { name: "Deselect all", exact: true }),
  ).toBeEnabled();
  await page.getByTitle("Music\\Album B", { exact: true }).click();
  await page.getByLabel("Filter library", { exact: true }).fill("two");
  await page.getByLabel(`Select ${b.path} from alice`, { exact: true }).check();
  const perFileDownload = page.getByRole("button", {
    name: "Download two.flac",
    exact: true,
  });
  await perFileDownload.scrollIntoViewIfNeeded();
  expect(
    await perFileDownload.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return r.left >= 0 && r.right <= innerWidth;
    }),
  ).toBe(true);
  await expect(page.locator(".results-selection")).toContainText(
    "2 selected · 3 KiB",
  );
  await expect(
    page.getByRole("button", {
      name: "Download selected from alice (top)",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Download selected from alice (bottom)",
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Download selected", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("Download queue is full");
  await expect(page.locator(".results-selection")).toContainText(
    "2 selected · 3 KiB",
  );
  await page.getByRole("button", { name: /^myself Unknown$/ }).click();
  await expect(
    page.getByRole("button", { name: "Deselect all", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByLabel("Select shared folder Music", { exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: /^alice Unknown$/ }).click();
  await expect(page.locator(".results-selection")).toContainText(
    "2 selected · 3 KiB",
  );
  reject = false;
  await page
    .getByRole("button", { name: "Download selected", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Deselect all", exact: true }),
  ).toBeDisabled();
  expect(batches[1].files).toHaveLength(2);
  await page
    .getByRole("button", { name: "Expand all folders", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Collapse all folders", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Expand all folders", exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: `../artifacts/persistent-share-selection-${info.project.name}.png`,
    fullPage: true,
  });
});

test("download storage protection can be changed and disabled in file locations", async ({
  page,
}, info) => {
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "File locations & library", exact: true })
    .click();
  const reserve = page.getByLabel("Download free-space reserve (GiB)");
  await expect(reserve).toHaveValue("1");
  await reserve.fill("0.5");
  await page
    .getByRole("button", { name: "Save storage protection", exact: true })
    .click();
  await expect
    .poll(() =>
      commands.some(
        (c) =>
          c.type === "update_storage_reserve" &&
          c.minimum_free_space_mib === 512,
      ),
    )
    .toBe(true);
  await reserve.fill("0");
  await page
    .getByRole("button", { name: "Save storage protection", exact: true })
    .click();
  await expect
    .poll(() =>
      commands.some(
        (c) =>
          c.type === "update_storage_reserve" && c.minimum_free_space_mib === 0,
      ),
    )
    .toBe(true);
  await pushRoomState(page, {
    ...emptySnapshot,
    network: { ...emptySnapshot.network, minimum_free_space_mib: 0 },
  });
  await expect(
    page.getByText("Download storage protection is disabled.", { exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `../artifacts/storage-protection-${info.project.name}.png`,
    fullPage: true,
  });
});

test("storage-held downloads show their reason, remain controllable offline and link to storage settings", async ({
  page,
}) => {
  const commands: Record<string, unknown>[] = [];
  const row = {
    username: "alice",
    path: "Album/track.flac",
    size: 1024,
    transferred: 512,
    speed: 0,
    status: "waiting_for_space",
    queue_position: null,
    detail: "Waiting for destination storage. Partial files are kept.",
  };
  await page.route("**/api/v1/transfers/downloads?**", (r) =>
    r.fulfill({
      json: {
        transfers: [row],
        total: 1,
        offset: 0,
        summary: { total: 1, active: 0, speed: 0, clearable: { all: 0 } },
      },
    }),
  );
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, { ...emptySnapshot, downloads: [row] });
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await expect(page.locator(".status-tag.waiting_for_space")).toHaveText(
    "Waiting for storage",
  );
  await expect(page.getByText(row.detail, { exact: true })).toBeVisible();
  await page.getByLabel("Pause track.flac", { exact: true }).click();
  await page.getByLabel("Cancel track.flac", { exact: true }).click();
  await expect
    .poll(() =>
      commands.filter((c) => c.type === "transfer").map((c) => c.action),
    )
    .toEqual(["pause", "cancel"]);
  await page
    .getByRole("button", { name: "Storage settings", exact: true })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Download storage protection",
      exact: true,
    }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("consecutive chat artwork keeps exact rows, one header and native clipboard text in rooms and private messages", async ({
  page,
  context,
}, info) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const art = [
    "░██████╗██████╗░░█████╗░███╗░░░███╗",
    "██╔════╝██╔══██╗██╔══██╗████╗░████║",
    "╚█████╗░██████╔╝███████║██╔████╔██║",
    "░╚═══██╗██╔═══╝░██╔══██║██║╚██╔╝██║",
    "██████╔╝██║░░░░░██║░░██║██║░╚═╝░██║",
    "╚═════╝░╚═╝░░░░░╚═╝░░╚═╝╚═╝░░░░░╚═╝",
  ];
  const username = "BeerNecessities";
  let state: Snapshot = {
    ...roomState(),
    room_unread: { club: 6 },
    room_read: { club: 0 },
    messages: [
      ...art.map((text, i) => ({
        id: i + 1,
        target: "club",
        username,
        text,
        private: false,
        timestamp: 1700000000 + i,
      })),
      ...art.map((text, i) => ({
        id: i + 101,
        target: username,
        username,
        text,
        private: true,
        timestamp: 1700000000 + i,
      })),
    ],
  };
  await page.route("**/api/v1/rooms/read", (r) =>
    r.fulfill({ status: 202, json: { status: "queued" } }),
  );
  await fixture(page, state);
  const previous = await page.evaluate(() => navigator.clipboard.readText());
  try {
    for (const section of ["Rooms", "Messages"]) {
      await page.getByRole("button", { name: section, exact: true }).click();
      if (section === "Messages")
        await page
          .getByRole("button", {
            name: `Open conversation with ${username}`,
            exact: true,
          })
          .click();
      const body = page.locator(".message-list .chat-artwork");
      await expect(page.locator(".message-list .message")).toHaveCount(1);
      await expect(page.locator(".message-list .message time")).toHaveCount(1);
      await expect(body.locator(":scope > span")).toHaveCount(6);
      expect(await body.textContent()).toBe(art.join("\n"));
      const geometry = await body.evaluate((el) => {
        const css = getComputedStyle(el);
        const rows = Array.from(el.querySelectorAll(":scope > span")).map(
          (row) => row.getBoundingClientRect().top,
        );
        return {
          lineHeight: parseFloat(css.lineHeight),
          fontSize: parseFloat(css.fontSize),
          whiteSpace: css.whiteSpace,
          deltas: rows.slice(1).map((top, i) => top - rows[i]),
        };
      });
      expect(geometry.whiteSpace).toBe("pre");
      expect(geometry.lineHeight).toBe(geometry.fontSize);
      for (const delta of geometry.deltas)
        expect(delta).toBeCloseTo(geometry.lineHeight, 1);
      await body.evaluate((el) => {
        const range = document.createRange();
        range.selectNodeContents(el);
        const selection = getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.keyboard.press("Control+c");
      await expect
        .poll(() =>
          page.evaluate(async () =>
            (await navigator.clipboard.readText()).replace(/\r\n/g, "\n"),
          ),
        )
        .toBe(art.join("\n"));
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await page.evaluate(() => getSelection()?.removeAllRanges());
      await page.locator(".message-list").screenshot({
        path: `../artifacts/chat-grouping-${section.toLowerCase()}-${info.project.name}.png`,
      });
    }
    await page.getByRole("button", { name: "Rooms", exact: true }).click();
    state = {
      ...state,
      messages: [
        ...state.messages,
        {
          id: 7,
          target: "club",
          username,
          text: "█".repeat(120),
          private: false,
          timestamp: 1700000006,
        },
      ],
    };
    await pushRoomState(page, state);
    const body = page.locator(".message-list .chat-artwork");
    await expect(body.locator(":scope > span")).toHaveCount(7);
    await expect(page.locator(".message-list .message")).toHaveCount(1);
    expect(await body.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(
      true,
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  } finally {
    await page.evaluate(
      (text) => navigator.clipboard.writeText(text),
      previous,
    );
  }
});

test("chat starts a new group for a different sender or a minute gap and preserves intentional blank lines", async ({
  page,
}) => {
  const state: Snapshot = {
    ...roomState(),
    messages: [
      {
        id: 1,
        target: "club",
        username: "alice",
        text: "First\n\nIntentional blank line",
        private: false,
        timestamp: 100,
      },
      {
        id: 2,
        target: "club",
        username: "alice",
        text: "Next :)",
        private: false,
        timestamp: 159,
      },
      {
        id: 3,
        target: "club",
        username: "bob",
        text: "Other sender",
        private: false,
        timestamp: 160,
      },
      {
        id: 4,
        target: "club",
        username: "bob",
        text: "After a minute",
        private: false,
        timestamp: 220,
      },
    ],
  };
  await page.route("**/api/v1/rooms/read", (r) =>
    r.fulfill({ status: 202, json: { status: "queued" } }),
  );
  await fixture(page, state);
  await page.getByRole("button", { name: "Rooms", exact: true }).click();
  await expect(page.locator(".message-list .message")).toHaveCount(3);
  expect(
    await page.locator(".message-list .chat-message").first().textContent(),
  ).toBe("First\n\nIntentional blank line\nNext 🙂");
  await expect(page.locator(".message-list .message time")).toHaveCount(3);
});

test("upload timeouts have readable reasons and can be filtered and cleared separately", async ({
  page,
}) => {
  const reason =
    "The receiving peer did not send its resume offset within 30 seconds.";
  const timed = {
    username: "alice",
    path: "Album/stalled.flac",
    size: 1024,
    transferred: 0,
    speed: 0,
    status: "timed_out",
    queue_position: null,
    detail: reason,
  };
  const active = {
    ...timed,
    path: "Album/active.flac",
    status: "uploading",
    detail: "",
    speed: 100,
  };
  const queries: string[] = [];
  const commands: unknown[] = [];
  await page.route("**/api/v1/transfers/uploads?**", (r) => {
    const status = new URL(r.request().url()).searchParams.get("status");
    queries.push(status || "all");
    return r.fulfill({
      json: {
        transfers: status === "timed_out" ? [timed] : [timed, active],
        total: status === "timed_out" ? 1 : 2,
        offset: 0,
        summary: {
          total: 2,
          active: 1,
          speed: 100,
          clearable: { all: 1, timed_out: 1 },
        },
      },
    });
  });
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, {
    ...emptySnapshot,
    connection: "connected",
    uploads: [timed, active],
    uploads_total: 2,
  });
  await page.getByRole("button", { name: "Uploads", exact: true }).click();
  await expect(page.locator(".status-tag.timed_out")).toHaveText("timed out");
  await expect(page.getByText(reason, { exact: true })).toBeVisible();
  await page
    .getByLabel("Filter transfer status", { exact: true })
    .selectOption("timed_out");
  await expect.poll(() => queries.includes("timed_out")).toBe(true);
  await expect(page.getByText("active.flac", { exact: true })).toHaveCount(0);
  await page
    .getByLabel("Clear transfer history status", { exact: true })
    .selectOption("timed_out");
  await page
    .getByRole("button", { name: "Clear 1 history records", exact: true })
    .click();
  await expect
    .poll(() => commands.at(-1))
    .toEqual({
      type: "clear_transfers",
      direction: "uploads",
      status: "timed_out",
    });
});

test("backup settings download encrypted files, validate a restore and require explicit confirmation", async ({
  page,
  context,
}, info) => {
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
  let ready: Record<string, unknown> | null = null;
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/backups", (r) => {
    if (r.request().method() === "POST") {
      const q = r.request().postDataJSON();
      commands.push(q);
      ready = {
        id: "export-id",
        kind: "export",
        bytes: 400100,
        summary,
        can_restart: true,
      };
      return r.fulfill({ json: ready });
    }
    return r.fulfill({ json: { busy: false, ready, can_restart: true } });
  });
  await context.route("**/api/v1/backups/export-id/file", (r) =>
    r.fulfill({
      body: "RustySoulBackup1-synthetic-encrypted-fixture",
      contentType: "application/octet-stream",
      headers: {
        "Content-Disposition": 'attachment; filename="rustysoul-backup.rsb"',
      },
    }),
  );
  await page.route("**/api/v1/backups/inspect", (r) => {
    expect(r.request().headers()["content-type"]).toContain(
      "multipart/form-data; boundary=",
    );
    ready = {
      id: "restore-id",
      kind: "restore",
      bytes: 400000,
      summary,
      can_restart: true,
    };
    return r.fulfill({ json: ready });
  });
  await page.route("**/api/v1/backups/restore-id/restore", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({
      json: { status: "restart_requested", administrator: "saved-admin" },
    });
  });
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "Backup & restore", exact: true })
    .click();
  await page
    .getByLabel("Current administrator password for backup", { exact: true })
    .fill("admin-password");
  await page
    .getByLabel("New backup passphrase", { exact: true })
    .fill("backup-password");
  await page
    .getByLabel("Confirm backup passphrase", { exact: true })
    .fill("backup-password");
  await page
    .getByRole("button", { name: "Create encrypted backup", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Backup ready", exact: true }),
  ).toBeVisible();
  const download = page.waitForEvent("download");
  await page
    .getByRole("link", { name: "Download encrypted backup", exact: true })
    .click();
  const saved = await download;
  expect(saved.suggestedFilename()).toBe("rustysoul-backup.rsb");
  expect(await saved.failure()).toBeNull();
  const fs = await import("node:fs/promises");
  expect(await fs.readFile((await saved.path())!, "utf8")).toBe(
    "RustySoulBackup1-synthetic-encrypted-fixture",
  );
  expect(commands[0]).toEqual({
    current_password: "admin-password",
    passphrase: "backup-password",
  });
  await expect(
    page.getByLabel("New backup passphrase", { exact: true }),
  ).toHaveValue("");
  await page
    .getByLabel("Encrypted backup file", { exact: true })
    .setInputFiles({
      name: "saved.rsb",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("encrypted backup"),
    });
  await page
    .getByLabel("Backup passphrase", { exact: true })
    .fill("backup-password");
  await page
    .getByRole("button", { name: "Validate backup", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Review before restoring", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Restore and restart", exact: true }),
  ).toBeDisabled();
  await expect(page.getByText("saved-admin", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page
    .locator(".backup-review")
    .last()
    .screenshot({
      path: `../artifacts/backup-review-${info.project.name}.png`,
    });
  await page
    .getByLabel("Current administrator password for restore", { exact: true })
    .fill("current-password");
  await page
    .getByLabel("Type RESTORE to confirm", { exact: true })
    .fill("RESTORE");
  await page
    .getByRole("button", { name: "Restore and restart", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Restoring RustySoul", exact: true }),
  ).toBeVisible();
  expect(commands.at(-1)).toEqual({
    current_password: "current-password",
    confirmation: "RESTORE",
  });
});
test("backup validation errors are dismissible and a different file hides a stale restore plan", async ({
  page,
}) => {
  const summary = {
    administrator: "saved-admin",
    created_at: 1700000000,
    database_bytes: 400000,
    schema: 11,
    downloads: 1,
    uploads: 0,
    indexed_files: 1,
    locations: { downloads: "/downloads", shares: ["/library"] },
  };
  await page.route("**/api/v1/backups", (r) =>
    r.fulfill({
      json: {
        busy: false,
        ready: {
          id: "old-plan",
          kind: "restore",
          bytes: 400000,
          summary,
          can_restart: true,
        },
        can_restart: true,
      },
    }),
  );
  await page.route("**/api/v1/backups/inspect", (r) =>
    r.fulfill({
      status: 400,
      json: { error: "Wrong passphrase or damaged backup" },
    }),
  );
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "Backup & restore", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Review before restoring", exact: true }),
  ).toBeVisible();
  await page
    .getByLabel("Encrypted backup file", { exact: true })
    .setInputFiles({
      name: "another.rsb",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("different backup"),
    });
  await expect(
    page.getByRole("button", { name: "Restore and restart", exact: true }),
  ).toHaveCount(0);
  await page
    .getByLabel("Backup passphrase", { exact: true })
    .fill("incorrect-password");
  await page
    .getByRole("button", { name: "Validate backup", exact: true })
    .click();
  await expect(
    page.getByText("Wrong passphrase or damaged backup", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Dismiss error", exact: true })
    .click();
  await expect(
    page.getByText("Wrong passphrase or damaged backup", { exact: true }),
  ).toHaveCount(0);
});

test("Soulseek privileges show status and recheck after the official purchase page", async ({
  page,
  context,
}, info) => {
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await context.route("https://www.slsknet.org/**", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<h1>Official purchase page fixture</h1>",
    }),
  );
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    username: "fixture + listener",
    credentials_configured: true,
    privileges: {
      official_server: true,
      remaining_seconds: 0,
      checked_at: 1700000000,
      expires_at: null,
      pending: false,
      watching: false,
      error: null,
    },
  };
  await fixture(page, state);
  await page
    .getByRole("button", {
      name: "Soulseek privileges: No active privileges",
      exact: true,
    })
    .click();
  const panel = page.locator(".privileges-panel");
  await expect(
    panel.getByText("No active privileges", { exact: true }),
  ).toBeVisible();
  const buy = panel.getByRole("link", { name: "Get Soulseek privileges" });
  expect(
    new URL((await buy.getAttribute("href"))!).searchParams.get("username"),
  ).toBe(state.username);
  const popupEvent = page.waitForEvent("popup");
  await buy.click();
  const popup = await popupEvent;
  await expect(popup.getByRole("heading")).toHaveText(
    "Official purchase page fixture",
  );
  await popup.close();
  await page.bringToFront();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect
    .poll(
      () =>
        commands.filter(
          (c) =>
            c.type === "check_privileges" && c.watch_for_activation === true,
        ).length,
    )
    .toBe(2);
  state.privileges = {
    ...state.privileges!,
    remaining_seconds: 259200,
    expires_at: 1700259200,
    checked_at: 1700000001,
  };
  await pushRoomState(page, state);
  await expect(
    panel.getByText("Privileges active", { exact: true }),
  ).toBeVisible();
  await expect(panel.getByText("3 days", { exact: true })).toBeVisible();
  await expect(
    panel.getByRole("link", { name: "Extend Soulseek privileges" }),
  ).toBeVisible();
  await panel.screenshot({
    path: `../artifacts/privileges-${info.project.name}.png`,
  });
});
test("Soulseek privileges distinguish offline unknown and failed checks", async ({
  page,
}) => {
  const state: Snapshot = {
    ...emptySnapshot,
    username: "fixture-self",
    credentials_configured: true,
  };
  await fixture(page, state);
  await page
    .getByRole("button", {
      name: "Soulseek privileges: Privileges not checked",
      exact: true,
    })
    .click();
  const panel = page.locator(".privileges-panel");
  await expect(
    panel.getByRole("button", { name: "Refresh privileges" }),
  ).toBeDisabled();
  await expect(panel.getByRole("link", { name: /Get|Extend/ })).toHaveCount(0);
  state.connection = "connected";
  state.privileges = {
    official_server: true,
    remaining_seconds: null,
    checked_at: null,
    expires_at: null,
    pending: true,
    watching: false,
    error: null,
  };
  await pushRoomState(page, state);
  await expect(
    panel.getByRole("button", { name: "Checking privileges…" }),
  ).toBeDisabled();
  state.privileges.pending = false;
  state.privileges.error =
    "Soulseek did not confirm privilege status. Refresh to try again.";
  await pushRoomState(page, state);
  await expect(panel.getByRole("alert")).toContainText("did not confirm");
  await expect(
    panel.getByText("No active privileges", { exact: true }),
  ).toHaveCount(0);
  await panel.getByRole("button", { name: "Dismiss error" }).click();
  await expect(panel.getByRole("alert")).toHaveCount(0);
});

test("scheduled library scans stay quiet while other scan banners and mandatory details remain", async ({
  page,
}) => {
  const state: Snapshot = {
    ...emptySnapshot,
    library: {
      ...emptySnapshot.library,
      scanning: true,
      phase: "scanning",
      trigger: "scheduled",
      processed: 44,
      indexed: 300,
    },
  };
  await fixture(page, state);
  await expect(
    page.getByRole("status", { name: "Library update banner" }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "File locations & library", exact: true })
    .click();
  await expect(
    page.getByRole("status", { name: "Library update details" }),
  ).toBeVisible();
  for (const trigger of ["manual", "filesystem", "startup", "locations"]) {
    state.library.trigger = trigger;
    await pushRoomState(page, state);
    await expect(
      page.getByRole("status", { name: "Library update banner" }),
    ).toBeVisible();
  }
  state.library.trigger = "scheduled";
  await pushRoomState(page, state);
  await expect(
    page.getByRole("status", { name: "Library update banner" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("status", { name: "Library update details" }),
  ).toBeVisible();
});
test("database maintenance queues a restart and shows space reclaimed for the completed job", async ({
  page,
}, info) => {
  let pending = false,
    done = false;
  const actions: object[] = [];
  await page.route("**/api/v1/maintenance", (r) => {
    if (r.request().method() === "POST") {
      actions.push(r.request().postDataJSON());
      pending = true;
      setTimeout(() => {
        pending = false;
        done = true;
      }, 400);
      return r.fulfill({
        json: { status: "restart_requested", id: "maintenance-job" },
      });
    }
    return r.fulfill({
      json: {
        can_restart: true,
        busy: false,
        pending,
        pending_id: pending ? "maintenance-job" : null,
        stats: {
          database_bytes: done ? 1048576 : 9437184,
          wal_bytes: 0,
          reusable_bytes: done ? 0 : 8388608,
        },
        stats_error: null,
        last_result: done
          ? {
              id: "maintenance-job",
              action: "compact",
              status: "completed",
              message: "Database compacted and query statistics optimized.",
              before_bytes: 9437184,
              after_bytes: 1048576,
              reclaimed_bytes: 8388608,
              elapsed_ms: 1500,
              at: 1700000000,
            }
          : null,
      },
    });
  });
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page
    .getByRole("tab", { name: "Database maintenance", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Database maintenance", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Compact database and restart" })
    .click();
  await expect(
    page.getByRole("button", { name: "Optimize query performance" }),
  ).toBeDisabled();
  await expect(
    page.getByText("Database compacted and query statistics optimized."),
  ).toBeVisible();
  expect(actions).toEqual([{ action: "compact" }]);
  await expect(
    page.getByRole("button", { name: "Optimize query performance" }),
  ).toBeEnabled();
  await page
    .locator(".maintenance-panel")
    .screenshot({ path: `../artifacts/maintenance-${info.project.name}.png` });
});
test("idle Away settings persist and a UI click reports activity immediately while automatically Away", async ({
  page,
}) => {
  const profile = {
    description: "",
    picture: null,
    likes: [],
    dislikes: [],
    away_on_startup: false,
    away_message: "",
    auto_away_minutes: 0,
  };
  const posts: Record<string, unknown>[] = [];
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/profile", (r) => {
    if (r.request().method() === "POST") {
      posts.push(r.request().postDataJSON());
      return r.fulfill({ status: 202, json: { status: "queued" } });
    }
    return r.fulfill({ json: profile });
  });
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    username: "fixture-self",
    profile: { ...profile, has_picture: false, revision: 0 },
  };
  await fixture(page, state);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "Profile", exact: true }).click();
  await page
    .getByRole("checkbox", { name: "Automatically set Away when idle" })
    .check();
  await page.getByLabel("Idle timeout (minutes)").fill("1");
  await page.getByRole("button", { name: "Save profile", exact: true }).click();
  await expect.poll(() => posts.length).toBe(1);
  expect(posts[0].auto_away_minutes).toBe(1);
  state.profile = { ...state.profile!, auto_away_minutes: 1 };
  await pushRoomState(page, state);
  await expect
    .poll(() => commands.filter((c) => c.type === "ui_activity").length)
    .toBe(1);
  state.away = true;
  state.away_automatic = true;
  await pushRoomState(page, state);
  await expect(page.getByText("Away (idle)", { exact: true })).toBeVisible();
  await page.getByRole("heading", { name: "Your Soulseek profile" }).click();
  await expect
    .poll(() => commands.filter((c) => c.type === "ui_activity").length)
    .toBe(2);
  expect(commands.filter((c) => c.type === "ui_activity")[1]).toEqual({
    type: "ui_activity",
    idle_seconds: 0,
  });
  state.away = false;
  state.away_automatic = false;
  await pushRoomState(page, state);
  await expect(
    page.getByText("Network connected", { exact: true }),
  ).toBeVisible();
});

test("API access configures scoped keys without revealing them again and revokes integrations", async ({
  page,
}, info) => {
  let enabled = false;
  let keys: object[] = [];
  const requests: Record<string, unknown>[] = [];
  await page.route("**/api/v1/api-access", (r) => {
    if (r.request().method() === "POST") {
      const q = r.request().postDataJSON();
      requests.push(q);
      if (q.action === "set_enabled") enabled = q.enabled;
      if (q.action === "create") {
        const key = {
          id: "api-key",
          name: q.name,
          scope: q.scope,
          created_at: 1700000000,
          expires_at: null,
          last_used_at: null,
        };
        keys = [key];
        return r.fulfill({
          json: { key, token: "rsapi_one-time-browser-fixture" },
        });
      }
      if (q.action === "revoke") keys = [];
    }
    return r.fulfill({ json: { enabled, keys, maximum_keys: 32 } });
  });
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await page.getByRole("tab", { name: "API access", exact: true }).click();
  await page
    .getByLabel("Current administrator password", { exact: true })
    .fill("eightpwd");
  await page.getByLabel("Enable external API access").check();
  await page.getByLabel("Key name").fill("Download automation");
  await page.getByLabel("API permissions").selectOption("control");
  await page.getByLabel("Key expiry").selectOption("0");
  await page
    .getByRole("button", { name: "Create API key", exact: true })
    .click();
  await expect(page.getByLabel("New API key", { exact: true })).toHaveValue(
    "rsapi_one-time-browser-fixture",
  );
  expect(requests[1]).toEqual({
    action: "create",
    name: "Download automation",
    scope: "control",
    expires_in_days: 0,
    current_password: "eightpwd",
  });
  await page.getByRole("button", { name: "I saved this key" }).click();
  await expect(page.getByLabel("New API key", { exact: true })).toHaveCount(0);
  await page
    .locator(".api-access-panel")
    .screenshot({ path: `../artifacts/api-access-${info.project.name}.png` });
  await page
    .getByRole("button", { name: "Revoke key Download automation" })
    .click();
  await expect(page.getByText("No keys have been created.")).toBeVisible();
});
test("disliked interests explore a verified shared-dislike cohort instead of topic likers", async ({
  page,
}) => {
  const commands: Record<string, unknown>[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    username: "fixture-self",
    profile: {
      description: "",
      likes: [],
      dislikes: ["noise"],
      away_on_startup: false,
      away_message: "",
      has_picture: false,
      revision: 0,
    },
    discovery: {
      query: { mode: "dislikes", item: "noise" },
      recommendations: [{ item: "jazz", rating: 2 }],
      against: [{ item: "spam", rating: -2 }],
      people: [{ username: "fellow-disliker", weight: 1 }],
      recommendations_at: 1700000000,
      people_at: 1700000000,
      recommendations_pending: false,
      people_pending: false,
      recommendations_error: null,
      people_error: null,
      dislike_evidence: {
        checked: 3,
        matched: 1,
        cached: 0,
        requested: 3,
        timed_out: 0,
        limited: false,
        missing_sources: 0,
      },
    },
  };
  await fixture(page, state);
  await page.getByRole("button", { name: "Discover", exact: true }).click();
  await page
    .getByRole("button", { name: "People who dislike noise", exact: true })
    .click();
  await expect
    .poll(() => commands.filter((c) => c.type === "discover").length)
    .toBe(1);
  expect(commands.find((c) => c.type === "discover")!.query).toEqual({
    mode: "dislikes",
    item: "noise",
  });
  await expect(
    page.getByRole("heading", { name: /People who also dislike this/ }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "3 public profiles checked · 1 confirmed matches · 0 cached profiles",
    ),
  ).toBeVisible();
  await expect(page.getByText(/not a network-wide list/)).toBeVisible();
  await page
    .getByText("Other things they dislike (1)", { exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Explore spam", exact: true }),
  ).toBeVisible();
});

test("whole transfer user groups inherit collapse and expand defaults as new users load", async ({
  page,
}) => {
  const record = (username: string, i: number) => ({
    username,
    path: `${username}-${i}.flac`,
    size: 100,
    transferred: 100,
    speed: 0,
    status: "completed",
    queue_position: null,
  });
  for (const direction of ["uploads", "downloads"] as const) {
    const requests: URLSearchParams[] = [];
    let releaseDave!: () => void;
    const daveReady = new Promise<void>((resolve) => {
      releaseDave = resolve;
    });
    await page.route(`**/api/v1/transfers/${direction}?**`, async (r) => {
      const q = new URL(r.request().url()).searchParams;
      requests.push(q);
      const offset = Number(q.get("offset"));
      if (offset >= 290) await daveReady;
      // Initial user crosses the normal file budget. Subsequent users arrive
      // only after that entire group, and each group is represented once.
      const users =
        offset === 0
          ? ([["alice", 130]] as const)
          : offset === 130
            ? ([
                ["bob", 80],
                ["carol", 80],
              ] as const)
            : ([["dave", 120]] as const);
      const transfers = users.flatMap(([u, n]) =>
        Array.from({ length: n }, (_, i) => record(u, i)),
      );
      await r.fulfill({
        json: {
          transfers,
          total: 410,
          offset,
          next_offset: offset + transfers.length,
          summary: {
            total: 410,
            active: 0,
            speed: 0,
            clearable: { all: 410, completed: 410 },
          },
        },
      });
    });
    if (direction === "uploads")
      await fixture(page, {
        ...emptySnapshot,
        uploads_total: 410,
        downloads_total: 410,
        uploads: [],
        downloads: [],
      });
    await page
      .getByRole("button", {
        name: direction === "uploads" ? "Uploads" : "Downloads",
        exact: true,
      })
      .click();
    if (direction === "downloads")
      await page.getByLabel("Group by user", { exact: true }).check();
    await expect(
      page.getByRole("button", {
        name: `Collapse ${direction} for alice`,
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.locator(".upload-group-heading").first()).toContainText(
      "130 files",
    );
    await page
      .getByRole("button", { name: "Collapse all users", exact: true })
      .click();
    await expect(
      page.getByRole("button", {
        name: `Expand ${direction} for bob`,
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", {
        name: `Expand ${direction} for carol`,
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.locator(".transfer-row")).toHaveCount(0);
    expect(
      requests.some(
        (q) => q.get("group_users") === "true" && q.get("offset") === "130",
      ),
    ).toBe(true);
    await page
      .getByRole("button", { name: `Expand ${direction} for bob`, exact: true })
      .click();
    await expect(page.getByText("bob-0.flac", { exact: true })).toBeVisible();
    await expect(
      page.getByRole("button", {
        name: `Expand ${direction} for alice`,
        exact: true,
      }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Expand all users", exact: true })
      .click();
    // Keep the last response held until scrolling has requested it. Releasing
    // it first races removal of the sentinel against Playwright's locator.
    await page.locator(".infinite-sentinel").scrollIntoViewIfNeeded();
    await expect
      .poll(() => requests.some((q) => q.get("offset") === "290"))
      .toBe(true);
    releaseDave();
    await expect(
      page.getByRole("button", {
        name: `Collapse ${direction} for dave`,
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.locator(".upload-group")).toHaveCount(4);
    await expect(page.locator(".result-pagination")).toContainText(
      "410 loaded",
    );
  }
  await page.screenshot({
    path: `../artifacts/transfer-scroll-${test.info().project.name}.png`,
    fullPage: false,
  });
});

test("settings sections follow the requested order including all existing sections", async ({
  page,
}) => {
  await fixture(page, emptySnapshot);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await expect(
    page.getByRole("tablist", { name: "Settings sections" }).getByRole("tab"),
  ).toHaveText([
    "Accounts",
    "Profile",
    "Appearance",
    "People & sharing",
    "File locations & library",
    "Network settings",
    "Status & privacy",
    "Logs & activity",
    "API access",
    "Database maintenance",
    "Backup & restore",
  ]);
});

test("download and upload selection supports Ctrl, virtualized Shift ranges, per-user all and cancellation", async ({
  page,
}) => {
  const commands: any[] = [];
  await page.route("**/api/v1/commands", (r) => {
    commands.push(r.request().postDataJSON());
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const files = Array.from({ length: 150 }, (_, i) => ({
    username: "alice",
    path: `Music/${String(i).padStart(3, "0")}.flac`,
    size: 100,
    transferred: 0,
    speed: 0,
    status: "queued",
    queue_position: null,
  }));
  files.push({
    ...files[0],
    path: "Music/zzz-completed.flac",
    status: "completed",
    transferred: 100,
  });
  await fixture(page, {
    ...emptySnapshot,
    connection: "connected",
    downloads: files,
    uploads: files,
  });
  for (const direction of ["downloads", "uploads"] as const) {
    const singular = direction === "uploads" ? "upload" : "download";
    await page
      .getByRole("button", {
        name: direction === "uploads" ? "Uploads" : "Downloads",
        exact: true,
      })
      .click();
    if (direction === "downloads")
      await page.getByLabel("Group by user", { exact: true }).check();
    await page
      .getByLabel("Order transfers", { exact: true })
      .selectOption("name");
    await page
      .getByLabel(`Select ${singular} Music/000.flac for alice`, {
        exact: true,
      })
      .check();
    await page
      .getByRole("group", {
        name: `${direction === "uploads" ? "Upload" : "Download"} Music/005.flac for alice`,
        exact: true,
      })
      .getByText("005.flac", { exact: true })
      .click({ modifiers: ["Control"] });
    await expect(
      page.getByText(`2 ${direction} selected`, { exact: false }),
    ).toBeVisible();
    await page.locator(".virtual-result-block").nth(2).scrollIntoViewIfNeeded();
    await page
      .getByRole("group", {
        name: `${direction === "uploads" ? "Upload" : "Download"} Music/120.flac for alice`,
        exact: true,
      })
      .getByText("120.flac", { exact: true })
      .click({ modifiers: ["Shift"] });
    await expect(
      page.getByText(`116 ${direction} selected`, { exact: false }),
    ).toBeVisible();
    await page
      .getByRole("button", {
        name: `Cancel selected ${direction}`,
        exact: true,
      })
      .click();
    await expect
      .poll(() =>
        commands.find(
          (c) => c.type === `cancel_${direction}` && c.paths.length === 116,
        ),
      )
      .toBeTruthy();
    const batch = commands.find(
      (c) => c.type === `cancel_${direction}` && c.paths.length === 116,
    );
    expect(batch.paths).toEqual(
      Array.from(
        { length: 116 },
        (_, i) => `Music/${String(i + 5).padStart(3, "0")}.flac`,
      ),
    );
    await expect(
      page.getByText(`0 ${direction} selected`, { exact: false }),
    ).toBeVisible();
    await page
      .getByRole("button", {
        name: `Select all ${direction} for alice`,
        exact: true,
      })
      .click();
    await expect(
      page.getByText(`150 ${direction} selected`, { exact: false }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Clear selection for alice", exact: true })
      .click();
    await expect(
      page.getByText(`0 ${direction} selected`, { exact: false }),
    ).toBeVisible();
    await page
      .getByRole("button", {
        name: `Cancel all ${direction} for alice`,
        exact: true,
      })
      .click();
    expect(commands).toContainEqual({
      type: `cancel_${direction}`,
      username: "alice",
      paths: [],
    });
    expect(
      commands.some((c) => c.paths?.includes("Music/zzz-completed.flac")),
    ).toBe(false);
  }
  await page.screenshot({
    path: `../artifacts/transfer-selection-${test.info().project.name}.png`,
    fullPage: false,
  });
});

test("large transfer cancellation batches keep failed submissions selected for retry", async ({
  page,
}) => {
  const requests: any[] = [];
  let reject = true;
  await page.route("**/api/v1/commands", (r) => {
    const command = r.request().postDataJSON();
    requests.push(command);
    if (requests.length === 2 && reject)
      return r.fulfill({
        status: 503,
        json: { error: "Temporary cancellation failure" },
      });
    return r.fulfill({ status: 202, json: { status: "queued" } });
  });
  const downloads = Array.from({ length: 300 }, (_, i) => ({
    username: "alice",
    path: `Music/${String(i).padStart(3, "0")}.flac`,
    size: 100,
    transferred: 0,
    speed: 0,
    status: "queued",
    queue_position: null,
  }));
  await fixture(page, { ...emptySnapshot, connection: "connected", downloads });
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await page.getByLabel("Group by user", { exact: true }).check();
  await page
    .getByRole("button", {
      name: "Select all downloads for alice",
      exact: true,
    })
    .click();
  await page
    .getByRole("button", { name: "Cancel selected downloads", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "Temporary cancellation failure",
  );
  await expect(
    page.getByText("44 downloads selected", { exact: false }),
  ).toBeVisible();
  expect(requests[0].paths).toHaveLength(256);
  expect(requests[1].paths).toHaveLength(44);
  reject = false;
  await page
    .getByRole("button", { name: "Cancel selected downloads", exact: true })
    .click();
  await expect(
    page.getByText("0 downloads selected", { exact: false }),
  ).toBeVisible();
  expect(requests[2].paths).toEqual(requests[1].paths);
});

test("priority and manual queue controls work for uploads and downloads without altering selection", async ({
  page,
}) => {
  let state: Snapshot = { ...emptySnapshot, connection: "connected" };
  const files = ["a.flac", "b.flac", "c.flac"].map((path, i) => ({
    username: "alice",
    path,
    size: 100,
    transferred: 0,
    speed: 0,
    status: "queued",
    queue_position: i + 1,
    priority: 0,
    local_queue_order: i,
  }));
  state = { ...state, downloads: files, uploads: files };
  const commands: any[] = [];
  await page.route("**/api/v1/commands", async (r) => {
    const command = r.request().postDataJSON();
    commands.push(command);
    if (command.type === "update_transfer_queue") {
      const direction = command.direction as "downloads" | "uploads";
      let next = state[direction].map((t) => ({ ...t }));
      const target = next.find((t) => t.path === command.path)!;
      if (command.priority !== undefined) target.priority = command.priority;
      next.sort(
        (a, b) =>
          (b.priority ?? 0) - (a.priority ?? 0) ||
          (a.local_queue_order ?? 0) - (b.local_queue_order ?? 0),
      );
      if (command.movement) {
        const at = next.findIndex((t) => t.path === command.path);
        const [item] = next.splice(at, 1);
        next.splice(command.movement === "bottom" ? next.length : 0, 0, item);
      }
      next = next.map((t, i) => ({ ...t, local_queue_order: i }));
      state = { ...state, [direction]: next };
      await r.fulfill({ status: 202, json: { status: "queued" } });
      await pushRoomState(page, state);
      return;
    }
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  for (const direction of ["downloads", "uploads"] as const) {
    const singular = direction === "uploads" ? "upload" : "download";
    await page
      .getByRole("button", {
        name: direction === "uploads" ? "Uploads" : "Downloads",
        exact: true,
      })
      .click();
    await page
      .getByLabel(`Select ${singular} b.flac for alice`, { exact: true })
      .check();
    await page
      .getByLabel(`Select ${singular} c.flac for alice`, { exact: true })
      .check();
    const row = page.getByRole("group", {
      name: `${direction === "uploads" ? "Upload" : "Download"} b.flac for alice`,
      exact: true,
    });
    await row.locator(".transfer-queue-controls > summary").click();
    await expect(
      page.getByText(`2 ${direction} selected`, { exact: false }),
    ).toBeVisible();
    await page
      .getByRole("button", {
        name: "Set High priority for b.flac from alice",
        exact: true,
      })
      .click();
    await expect(
      page.getByLabel("Order transfers", { exact: true }),
    ).toHaveValue("local_queue");
    if (direction === "uploads")
      await expect(
        page.getByLabel("Group by user", { exact: true }),
      ).toBeChecked();
    else
      await expect(
        page.getByLabel("Group by user", { exact: true }),
      ).not.toBeChecked();
    await expect(page.locator(".transfer-detail > strong")).toHaveText([
      "b.flac",
      "a.flac",
      "c.flac",
    ]);
    await expect(
      page.getByText(`2 ${direction} selected`, { exact: false }),
    ).toBeVisible();
    await page
      .getByRole("button", {
        name: "Set Normal priority for b.flac from alice",
        exact: true,
      })
      .click();
    await page
      .getByRole("button", {
        name: "Move to bottom: b.flac from alice",
        exact: true,
      })
      .click();
    await expect(page.locator(".transfer-detail > strong")).toHaveText([
      "a.flac",
      "c.flac",
      "b.flac",
    ]);
    expect(commands).toContainEqual({
      type: "update_transfer_queue",
      direction,
      username: "alice",
      path: "b.flac",
      priority: 1,
    });
    expect(commands).toContainEqual({
      type: "update_transfer_queue",
      direction,
      username: "alice",
      path: "b.flac",
      movement: "bottom",
    });
  }
  await page.screenshot({
    path: `../artifacts/transfer-priority-${test.info().project.name}.png`,
    fullPage: false,
  });
});

test("partial byte counts stay visible through failure, retry waiting and reload", async ({
  page,
}) => {
  const files = ["failed", "timed_out", "retry_waiting", "queued"].map(
    (status) => ({
      username: "alice",
      path: `${status}.flac`,
      status,
      size: 1048576,
      transferred: 4093,
      speed: 0,
      queue_position: null,
    }),
  );
  let state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    downloads: files,
  };
  const commands: unknown[] = [];
  await page.route("**/api/v1/commands", async (r) => {
    const command = r.request().postDataJSON();
    commands.push(command);
    if (command.type === "transfer" && command.action === "resume") {
      state = {
        ...state,
        downloads: state.downloads.map((t) =>
          t.path === command.path ? { ...t, status: "queued" } : t,
        ),
      };
      await r.fulfill({ status: 202, json: { status: "queued" } });
      await pushRoomState(page, state);
      return;
    }
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state, true);
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await expect(page.locator(".transfer-progress-count")).toHaveCount(4);
  for (const file of files) {
    const row = page.getByRole("group", {
      name: `Download ${file.path} for alice`,
      exact: true,
    });
    await expect(row.locator(".transfer-progress-count")).toHaveText(
      "4 KiB of 1 MiB transferred",
    );
    await expect(row.getByRole("progressbar")).toHaveAttribute(
      "aria-valuetext",
      "4,093 of 1,048,576 bytes transferred",
    );
  }
  await page
    .getByRole("button", { name: "Retry failed.flac", exact: true })
    .click();
  expect(commands).toContainEqual({
    type: "transfer",
    username: "alice",
    path: "failed.flac",
    action: "resume",
  });
  await expect(
    page.getByRole("button", { name: "Pause failed.flac", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".transfer-progress-count")).toHaveText(
    Array(4).fill("4 KiB of 1 MiB transferred"),
  );
  await page.reload();
  await expect(page.locator(".transfer-progress-count")).toHaveText(
    Array(4).fill("4 KiB of 1 MiB transferred"),
  );
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `../artifacts/partial-progress-${test.info().project.name}.png`,
    fullPage: true,
  });
});

test("empty search results queue normally and completed empty files offer saving without playback", async ({
  page,
}) => {
  const file = {
    username: "alice",
    path: "Music/empty.txt",
    size: 0,
    free_slots: 1,
    speed: 0,
    attributes: {},
  };
  const state: Snapshot = {
    ...emptySnapshot,
    connection: "connected",
    searches: [
      {
        id: "empty-search",
        query: "empty",
        collecting: false,
        results: [file],
      },
    ],
  };
  let queued: unknown;
  await page.route("**/api/v1/downloads/batch*", async (r) => {
    queued = r.request().postDataJSON();
    await r.fulfill({ status: 202, json: { status: "queued" } });
  });
  await fixture(page, state);
  await expect(page.getByText("empty.txt", { exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "Download empty.txt", exact: true })
    .click();
  await expect
    .poll(() => queued)
    .toEqual({
      files: [{ username: "alice", path: "Music/empty.txt", size: 0 }],
    });
  await pushRoomState(page, {
    ...state,
    downloads: [
      {
        ...file,
        status: "completed",
        transferred: 0,
        queue_position: null,
        local_path: "alice/Music/empty.txt",
      },
    ],
  });
  await page.getByRole("button", { name: "Downloads", exact: true }).click();
  await expect(
    page.getByRole("progressbar", {
      name: "Progress for empty.txt",
      exact: true,
    }),
  ).toHaveAttribute("aria-valuenow", "100");
  await page
    .getByRole("button", { name: "Open empty.txt", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Empty file", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("This file is empty. You can save it to your device.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator("audio")).toHaveCount(0);
  await expect(
    page.getByRole("link", { name: "Save file", exact: true }),
  ).toHaveAttribute("href", /path=Music%2Fempty.txt.*download=true/);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `../artifacts/empty-file-${test.info().project.name}.png`,
    fullPage: false,
  });
  await page.getByRole("button", { name: "Close file", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Empty file", exact: true }),
  ).toHaveCount(0);
});
