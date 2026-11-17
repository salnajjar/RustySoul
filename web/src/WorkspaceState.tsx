import {
  createContext,
  useContext,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";
import type { FileResult } from "./types";
export function useChoice<T extends string>(
  key: string,
  initial: T,
  allowed: readonly string[],
) {
  const [value, setValue] = useState<T>(() => {
    try {
      const saved = localStorage.getItem(`rustysoul.${key}`);
      return saved && allowed.includes(saved) ? (saved as T) : initial;
    } catch {
      return initial;
    }
  });
  const set = (next: T) => {
    setValue(next);
    try {
      localStorage.setItem(`rustysoul.${key}`, next);
    } catch {
      /* Browser storage may be unavailable. */
    }
  };
  return [value, set] as const;
}
type Selections = Record<string, Map<string, FileResult>>;
const Context = createContext<{
  values: Selections;
  set: Dispatch<SetStateAction<Selections>>;
} | null>(null);
export function SelectionProvider({ children }: { children: ReactNode }) {
  const [values, set] = useState<Selections>({});
  return (
    <Context.Provider value={{ values, set }}>{children}</Context.Provider>
  );
}
export function useSelection(scope: string) {
  const context = useContext(Context);
  const [local, setLocal] = useState<Map<string, FileResult>>(new Map());
  const selected = context?.values[scope] || local;
  const setSelected: Dispatch<SetStateAction<Map<string, FileResult>>> = (
    value,
  ) => {
    if (context)
      context.set((old) => ({
        ...old,
        [scope]:
          typeof value === "function" ? value(old[scope] || new Map()) : value,
      }));
    else setLocal(value);
  };
  return [selected, setSelected] as const;
}
export const fileIdentity = (file: Pick<FileResult, "username" | "path">) =>
  JSON.stringify([file.username, file.path]);
export async function accepted(
  send: (command: object) => unknown,
  command: object,
) {
  return (await send(command)) !== false;
}
