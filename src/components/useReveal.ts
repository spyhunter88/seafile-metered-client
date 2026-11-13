import { useCallback } from "react";
import { api, errorMessage } from "../api";
import { useToast } from "./Toasts";

/** Shows a library file/folder in Explorer; a deleted item opens its closest parent folder. */
export function useReveal() {
  const toast = useToast();
  return useCallback(
    async (repoId: string, path: string | null) => {
      try {
        const res = await api.revealLibraryPath(repoId, path);
        if (res === "Missing" && path) {
          toast("info", "Item no longer exists", `Opened the closest folder of ${path}`);
        }
      } catch (e) {
        toast("error", "Cannot open in Explorer", errorMessage(e));
      }
    },
    [toast],
  );
}
