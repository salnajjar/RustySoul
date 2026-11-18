import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import FolderTree from "./FolderTree";
import { api } from "./api";

vi.mock("./api", () => ({ api: vi.fn() }));
const request = vi.mocked(api);
const props = {
  token: "",
  username: "alice",
  revision: 1,
  filter: "",
  selected: null,
  select: vi.fn(),
};
const folders = [
  {
    path: "Music",
    parent: "",
    name: "Music",
    files: 151,
    direct_files: 0,
    has_children: true,
  },
  ...Array.from({ length: 151 }, (_, i) => ({
    path: `Music\\Album ${i}`,
    parent: "Music",
    name: `Album ${i}`,
    files: 1,
    direct_files: 1,
    has_children: false,
  })),
];
beforeEach(() => {
  request.mockReset();
});
afterEach(cleanup);

it("loads every folder once and expands and filters without more requests", async () => {
  request.mockResolvedValue({ folders, total: folders.length, offset: 0 });
  const { rerender } = render(<FolderTree {...props} />);
  await screen.findByTitle("Music");
  expect(request).toHaveBeenCalledTimes(1);
  expect(request.mock.calls[0][1]).toBe("browse/tree?username=alice&all=true");
  fireEvent.click(screen.getByRole("button", { name: "Expand all folders" }));
  expect(screen.getAllByRole("treeitem")).toHaveLength(152);
  expect(screen.getByTitle("Music\\Album 150")).toBeInTheDocument();
  rerender(<FolderTree {...props} filter="album 150" />);
  expect(screen.getAllByRole("treeitem")).toHaveLength(2);
  expect(screen.getByTitle("Music")).toBeInTheDocument();
  expect(screen.getByTitle("Music\\Album 150")).toBeInTheDocument();
  expect(request).toHaveBeenCalledTimes(1);
  expect(screen.queryByText(/Load more/)).not.toBeInTheDocument();
});

it("rejects incomplete listings and retries the complete request", async () => {
  request.mockResolvedValueOnce({
    folders: folders.slice(0, 1),
    total: 152,
    offset: 0,
  });
  render(<FolderTree {...props} />);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "complete folder tree was not returned",
  );
  expect(screen.queryByTitle("Music")).not.toBeInTheDocument();
  request.mockResolvedValueOnce({ folders, total: 152, offset: 0 });
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await screen.findByTitle("Music");
  expect(request).toHaveBeenCalledTimes(2);
});

it("aborts previous requests and ignores stale responses when changing peers or revisions", async () => {
  let resolveFirst!: (data: unknown) => void;
  request.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        resolveFirst = resolve;
      }),
  );
  request.mockResolvedValue({
    folders: [{ ...folders[1], parent: "", path: "Bob", name: "Bob" }],
    total: 1,
    offset: 0,
  });
  const { rerender } = render(<FolderTree {...props} />);
  const signal = request.mock.calls[0][3];
  rerender(<FolderTree {...props} username="bob" />);
  expect(signal?.aborted).toBe(true);
  await screen.findByTitle("Bob");
  await act(async () => {
    resolveFirst({ folders, total: 152, offset: 0 });
  });
  expect(screen.queryByTitle("Music")).not.toBeInTheDocument();
  rerender(<FolderTree {...props} username="bob" revision={2} />);
  await waitFor(() => expect(request).toHaveBeenCalledTimes(3));
  await screen.findByTitle("Bob");
});
