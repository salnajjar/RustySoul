import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";

interface InstallPrompt extends Event {
  prompt: () => Promise<unknown>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
}
interface Installation {
  installed: boolean;
  secure: boolean;
  ready: boolean;
  busy: boolean;
  error: string;
  install: () => Promise<void>;
}
const InstallationContext = createContext<Installation | null>(null);

// Mount above App so a prompt arriving on the sign-in/search page is retained
// until the user opens Appearance. Nothing sensitive is persisted or cached.
export function InstallAppProvider({ children }: { children: ReactNode }) {
  const [prompt, setPrompt] = useState<InstallPrompt | null>(null);
  const [installed, setInstalled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const secure = window.isSecureContext === true;
  useEffect(() => {
    const media = window.matchMedia?.("(display-mode: standalone)");
    const update = () =>
      setInstalled(
        !!media?.matches ||
          (navigator as Navigator & { standalone?: boolean }).standalone ===
            true,
      );
    const available = (event: Event) => {
      if (!secure || typeof (event as InstallPrompt).prompt !== "function")
        return;
      event.preventDefault();
      setPrompt(event as InstallPrompt);
      setError("");
    };
    const complete = () => {
      setInstalled(true);
      setPrompt(null);
      setError("");
    };
    update();
    window.addEventListener("beforeinstallprompt", available);
    window.addEventListener("appinstalled", complete);
    media?.addEventListener("change", update);
    return () => {
      window.removeEventListener("beforeinstallprompt", available);
      window.removeEventListener("appinstalled", complete);
      media?.removeEventListener("change", update);
    };
  }, [secure]);
  const install = async () => {
    if (!prompt || busy) return;
    setBusy(true);
    setError("");
    try {
      await prompt.prompt();
      await prompt.userChoice;
    } catch {
      setError(
        "Could not open the install prompt. Try your browser’s install menu instead.",
      );
    } finally {
      // A browser prompt may only be used once, including after dismissal.
      setPrompt(null);
      setBusy(false);
    }
  };
  return (
    <InstallationContext.Provider
      value={{ installed, secure, ready: !!prompt, busy, error, install }}
    >
      {children}
    </InstallationContext.Provider>
  );
}

export default function InstallApp() {
  const state = useContext(InstallationContext);
  if (!state) return null;
  return (
    <div className="install-app">
      <h3>RustySoul as an app</h3>
      {state.installed ? (
        <p>RustySoul is installed on this device.</p>
      ) : (
        <>
          <p>Give your workspace its own window and home-screen icon.</p>
          {!state.secure ? (
            <p>
              For installation, open RustySoul over HTTPS or through localhost.
              A plain HTTP address on your LAN does not support app
              installation.
            </p>
          ) : state.ready ? (
            <button
              className="button small secondary"
              disabled={state.busy}
              onClick={() => void state.install()}
            >
              {state.busy ? "Opening install prompt…" : "Install RustySoul"}
            </button>
          ) : (
            <p>
              Use your browser’s Install app or Add to Home Screen option, where
              available. On iPhone or iPad, open Safari, tap Share, then Add to
              Home Screen.
            </p>
          )}
          <p>
            Your daemon must remain reachable. Transfers keep running on the
            server when you close the app.
          </p>
        </>
      )}
      {state.error && <p role="status">{state.error}</p>}
    </div>
  );
}
