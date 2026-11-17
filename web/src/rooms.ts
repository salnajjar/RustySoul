export interface SortableRoom {
  name: string;
  users: number;
  last_activity?: number | null;
}
export type RoomSort = "users" | "name" | "activity";
export function sortRooms<T extends SortableRoom>(
  rooms: T[],
  sort: RoomSort,
): T[] {
  return [...rooms].sort(
    (a, b) =>
      (sort === "name"
        ? a.name.localeCompare(b.name)
        : sort === "activity"
          ? (b.last_activity ?? -1) - (a.last_activity ?? -1)
          : b.users - a.users) || a.name.localeCompare(b.name),
  );
}
