import { useCallback, useEffect, useRef, useState } from "react";

import { api, subscribe } from "../lib/ipc";
import { toAppError, type Settings } from "../lib/types";
import { useToasts } from "./toasts";

export type UpdateSettings = (mutate: (draft: Settings) => void) => Promise<boolean>;

/**
 * Loads settings and saves every change through the backend, which
 * validates and applies side effects. Saves are serialised so rapid edits
 * can never land out of order; a rejected save rolls the UI back.
 */
export function useSettings(): { settings: Settings | null; update: UpdateSettings } {
  const [settings, setSettings] = useState<Settings | null>(null);
  const latest = useRef<Settings | null>(null);
  const saveChain = useRef<Promise<unknown>>(Promise.resolve());
  const { push } = useToasts();

  const apply = useCallback((next: Settings) => {
    latest.current = next;
    setSettings(next);
  }, []);

  useEffect(() => {
    api
      .getSettings()
      .then(apply)
      .catch((err: unknown) => push("error", toAppError(err).message));
    return subscribe("settings:changed", apply);
  }, [apply, push]);

  const update = useCallback<UpdateSettings>(
    (mutate) => {
      const run = async (): Promise<boolean> => {
        const previous = latest.current;
        if (!previous) return false;
        const draft = structuredClone(previous);
        mutate(draft);
        apply(draft);
        try {
          apply(await api.saveSettings(draft));
          return true;
        } catch (err) {
          apply(previous);
          push("error", toAppError(err).message);
          return false;
        }
      };
      const result = saveChain.current.then(run, run);
      saveChain.current = result;
      return result;
    },
    [apply, push],
  );

  return { settings, update };
}
