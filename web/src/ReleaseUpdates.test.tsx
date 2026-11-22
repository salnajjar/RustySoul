import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import ReleaseNotice, {
  ReleaseSettings,
  type ReleaseStatus,
} from "./ReleaseUpdates";
import RetrySettings from "./RetrySettings";
import { api } from "./api";
import { emptySnapshot } from "./types";
vi.mock("./api", () => ({ api: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  localStorage.clear();
});
const release: ReleaseStatus = {
  current_version: "0.1.0",
  install_type: "podman",
  enabled: true,
  checking: false,
  checked_at: 100,
  next_check_at: 21700,
  error: null,
  available: {
    version: "0.2.0",
    release_url: "https://github.com/salnajjar/RustySoul/releases/tag/v0.2.0",
    instructions_url:
      "https://github.com/salnajjar/RustySoul/blob/v0.2.0/docs/CONTAINERS.md",
    image: "ghcr.io/salnajjar/rustysoul:0.2.0",
    native_url: null,
    checksum_url: null,
  },
};
it("links to Podman instructions and remembers dismissal for just that version", async () => {
  vi.mocked(api).mockResolvedValue(release);
  const first = render(<ReleaseNotice token="fixture" enabled />);
  expect(
    await screen.findByText("RustySoul 0.2.0 is available."),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Update Podman" })).toHaveAttribute(
    "href",
    release.available!.instructions_url,
  );
  expect(
    screen.getByText("ghcr.io/salnajjar/rustysoul:0.2.0"),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Dismiss message" }));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  first.unmount();
  const second = render(<ReleaseNotice token="fixture" enabled />);
  await waitFor(() => expect(api).toHaveBeenCalledTimes(2));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  second.unmount();
  vi.mocked(api).mockResolvedValue({
    ...release,
    available: { ...release.available, version: "0.3.0" },
  });
  render(<ReleaseNotice token="fixture" enabled />);
  expect(
    await screen.findByText("RustySoul 0.3.0 is available."),
  ).toBeInTheDocument();
});
it("links native installs to the versioned binary and checksum", async () => {
  const native = {
    ...release,
    install_type: "native",
    available: {
      ...release.available,
      image: null,
      native_url:
        "https://github.com/salnajjar/RustySoul/releases/download/v0.2.0/rustysoul-linux-x86_64.tar.gz",
      checksum_url:
        "https://github.com/salnajjar/RustySoul/releases/download/v0.2.0/rustysoul-linux-x86_64.tar.gz.sha256",
    },
  };
  vi.mocked(api).mockResolvedValue(native);
  render(<ReleaseNotice token="fixture" enabled />);
  expect(
    await screen.findByRole("link", { name: "Download Linux update" }),
  ).toHaveAttribute("href", native.available.native_url);
  expect(
    screen.getByRole("link", { name: "SHA-256 checksum" }),
  ).toHaveAttribute("href", native.available.checksum_url);
});
it("stays quiet when checks are disabled or unavailable", async () => {
  vi.mocked(api).mockRejectedValue(new Error("Offline"));
  const { rerender } = render(
    <ReleaseNotice token="fixture" enabled={false} />,
  );
  expect(api).not.toHaveBeenCalled();
  rerender(<ReleaseNotice token="fixture" enabled />);
  await waitFor(() => expect(api).toHaveBeenCalledTimes(1));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
it("saves release checking without replacing other network preferences", async () => {
  vi.mocked(api).mockResolvedValue(release);
  render(<ReleaseSettings token="fixture" state={emptySnapshot} />);
  fireEvent.click(
    screen.getByLabelText("Check for new releases automatically"),
  );
  await waitFor(() =>
    expect(api).toHaveBeenCalledWith("fixture", "commands", {
      type: "update_network",
      settings: { ...emptySnapshot.network, check_for_updates: false },
    }),
  );
});
it("keeps automatic download and upload retry controls independent", async () => {
  vi.mocked(api).mockResolvedValue({});
  render(<RetrySettings token="fixture" state={emptySnapshot} />);
  expect(
    screen.getByLabelText(
      "Automatically retry interrupted or timed-out uploads",
    ),
  ).toBeChecked();
  fireEvent.click(
    screen.getByLabelText(
      "Automatically retry interrupted or timed-out uploads",
    ),
  );
  fireEvent.click(screen.getByRole("button", { name: "Save retry settings" }));
  await waitFor(() =>
    expect(api).toHaveBeenCalledWith("fixture", "commands", {
      type: "update_retry_policy",
      settings: { ...emptySnapshot.retry_policy, uploads_enabled: false },
    }),
  );
});
