import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import Discover from "./Discover";
import { emptySnapshot, type DiscoveryView } from "./types";
afterEach(cleanup);
const data: DiscoveryView = {
  query: { mode: "personal" },
  recommendations: [],
  against: [],
  people: [],
  recommendations_at: 1,
  people_at: 1,
  recommendations_pending: false,
  people_pending: false,
  recommendations_error: null,
  people_error: null,
};
it("renders hostile cached topics as text and keeps offline network actions disabled", () => {
  const topic = "<img src=x onerror=alert(1)>";
  const send = vi.fn(),
    signin = vi.fn();
  const { container } = render(
    <Discover
      state={{
        ...emptySnapshot,
        discovery: { ...data, recommendations: [{ item: topic, rating: 3 }] },
      }}
      send={send}
      search={vi.fn()}
      profile={vi.fn()}
      signin={signin}
    />,
  );
  expect(
    screen.getByRole("button", { name: `Explore ${topic}` }),
  ).toHaveTextContent(topic);
  expect(container.querySelector("img")).toBeNull();
  expect(
    screen.getByRole("button", { name: `Search files for ${topic}` }),
  ).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Refresh discovery" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: `Like ${topic}` }));
  expect(send).toHaveBeenCalledWith({
    type: "set_interest",
    item: topic,
    disposition: "like",
  });
  fireEvent.click(screen.getByRole("button", { name: "Sign in to Soulseek" }));
  expect(signin).toHaveBeenCalledOnce();
});
it("limits rendered discovery rows and filters across entries that are not rendered yet", () => {
  const { container } = render(
    <Discover
      state={{
        ...emptySnapshot,
        connection: "connected",
        discovery: {
          ...data,
          recommendations: Array.from({ length: 500 }, (_, n) => ({
            item: `topic-${n}`,
            rating: 500 - n,
          })),
        },
      }}
      send={vi.fn()}
      search={vi.fn()}
      profile={vi.fn()}
      signin={vi.fn()}
    />,
  );
  expect(container.querySelectorAll(".discovery-item")).toHaveLength(50);
  fireEvent.click(
    screen.getByRole("button", { name: "Show more recommendations" }),
  );
  expect(container.querySelectorAll(".discovery-item")).toHaveLength(100);
  fireEvent.change(screen.getByLabelText("Filter discovery results"), {
    target: { value: "topic-499" },
  });
  expect(container.querySelectorAll(".discovery-item")).toHaveLength(1);
  expect(
    screen.getByRole("button", { name: "Explore topic-499" }),
  ).toBeInTheDocument();
});

it("explores own dislikes with the correct polarity and labels only verified cohort results", () => {
  const send = vi.fn();
  render(
    <Discover
      state={{
        ...emptySnapshot,
        connection: "connected",
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
          ...data,
          query: { mode: "dislikes", item: "noise" },
          people: [{ username: "fellow-disliker", weight: 1 }],
          recommendations: [{ item: "jazz", rating: 2 }],
          against: [{ item: "spam", rating: -2 }],
          dislike_evidence: {
            checked: 20,
            matched: 1,
            cached: 2,
            requested: 18,
            timed_out: 0,
            limited: false,
            missing_sources: 0,
          },
        },
      }}
      send={send}
      search={vi.fn()}
      profile={vi.fn()}
      signin={vi.fn()}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "People who dislike noise" }),
  );
  expect(send).toHaveBeenCalledWith({
    type: "discover",
    query: { mode: "dislikes", item: "noise" },
    refresh: false,
  });
  expect(
    screen.getByRole("heading", { name: /People who also dislike this/ }),
  ).toBeVisible();
  expect(screen.getByText(/20 public profiles checked/)).toBeVisible();
  expect(screen.getByText(/not a network-wide list/)).toBeVisible();
});
