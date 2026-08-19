import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { getLang, has, t as translate } from "./i18n";

/** The catalog is settled before the first render, so this never changes. */
export function useT() {
  return translate;
}

/** Attribute label from the catalog, falling back to the raw Cognito name. */
export function useLabel() {
  const t = useT();
  return useCallback(
    (name: string) => {
      const key = `attr.${name}`;
      return has(key) ? t(key) : name.replace(/^(custom|dev):/, "");
    },
    [t],
  );
}

export function useDateFormat() {
  return useCallback((iso: string | null) => {
    if (!iso) return "—";
    const date = new Date(iso);
    if (Number.isNaN(date.getTime())) return "—";
    return new Intl.DateTimeFormat(getLang(), {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(date);
  }, []);
}

export interface Toast {
  id: number;
  message: string;
  kind: "ok" | "error";
}

export interface ToastApi {
  toasts: Toast[];
  notify: (message: string, kind?: Toast["kind"]) => void;
  dismiss: (id: number) => void;
}

export const ToastContext = createContext<ToastApi | null>(null);

export function useToast(): ToastApi {
  const context = useContext(ToastContext);
  if (!context) throw new Error("ToastContext missing");
  return context;
}

export function useToastState(): ToastApi {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const dismiss = useCallback((id: number) => {
    setToasts((list) => list.filter((toast) => toast.id !== id));
  }, []);

  const notify = useCallback(
    (message: string, kind: Toast["kind"] = "ok") => {
      const id = Date.now() + Math.random();
      setToasts((list) => [...list, { id, message, kind }]);
      setTimeout(() => dismiss(id), 6000);
    },
    [dismiss],
  );

  return { toasts, notify, dismiss };
}

export const RouterContext = createContext<(to: string) => void>(() => {});

export function useNavigate() {
  return useContext(RouterContext);
}

/** Minimal history routing; the server serves the shell for any non-asset path. */
export function useLocation() {
  const [path, setPath] = useState(() => window.location.pathname);

  useEffect(() => {
    const onPop = () => setPath(window.location.pathname);
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const navigate = useCallback((to: string, replace = false) => {
    if (to === window.location.pathname) return;
    if (replace) window.history.replaceState({}, "", to);
    else window.history.pushState({}, "", to);
    setPath(to);
  }, []);

  return { path, navigate };
}
