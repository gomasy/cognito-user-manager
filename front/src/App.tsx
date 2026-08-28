import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api, UNAUTHORIZED_EVENT, UnauthorizedError } from "./api";
import {
  RouterContext,
  ToastContext,
  errorText,
  useLocation,
  useT,
  useToastState,
} from "./hooks";
import type { PoolInfo, SessionInfo } from "./types";
import { Account } from "./components/Account";
import { AdminGroupDetail } from "./components/AdminGroupDetail";
import { AdminGroups } from "./components/AdminGroups";
import { AdminUserCreate } from "./components/AdminUserCreate";
import { AdminUserDetail } from "./components/AdminUserDetail";
import { AdminUsers } from "./components/AdminUsers";
import { Layout } from "./components/Layout";
import { Login } from "./components/Login";
import { Toasts } from "./components/Toast";

type Boot = "loading" | "signedOut" | "ready";

export function App() {
  const t = useT();
  const toast = useToastState();
  const { path, navigate } = useLocation();
  const [boot, setBoot] = useState<Boot>("loading");
  const [session, setSession] = useState<SessionInfo | null>(null);
  const [pool, setPool] = useState<PoolInfo | null>(null);

  const load = useCallback(async () => {
    try {
      const [current, poolInfo] = await Promise.all([api.session(), api.pool()]);
      setSession(current);
      setPool(poolInfo);
      setBoot("ready");
    } catch (e) {
      if (e instanceof UnauthorizedError) setBoot("signedOut");
      else {
        toast.notify(errorText(e), "error");
        setBoot("signedOut");
      }
    }
  }, [toast]);

  // Boots once; later refreshes go through the sign-in / sign-out handlers.
  useEffect(() => {
    void load();
  }, []);

  // Creating or deleting a group changes the list the user screens assign
  // from, which is loaded once at boot; a failure here is not worth a toast of
  // its own, since the action that triggered it reported its own outcome.
  const reloadPool = useCallback(async () => {
    try {
      setPool(await api.pool());
    } catch {
      // The action that changed the groups reported its own outcome already.
    }
  }, []);

  // A session can lapse mid-visit, so any 401 drops back to the sign-in screen.
  useEffect(() => {
    const onUnauthorized = () => {
      setSession(null);
      setPool(null);
      setBoot("signedOut");
    };
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
  }, []);

  const signOut = async () => {
    await api.logout().catch(() => undefined);
    setSession(null);
    setPool(null);
    setBoot("signedOut");
    navigate("/", true);
  };

  if (boot === "loading") {
    return <main className="page page--narrow"><p className="hint">{t("common.loading")}</p></main>;
  }

  if (boot === "signedOut" || !session || !pool) {
    return (
      <ToastContext.Provider value={toast}>
        <Login
          onSignedIn={() => {
            setBoot("loading");
            void load();
          }}
        />
        <Toasts toasts={toast.toasts} dismiss={toast.dismiss} />
      </ToastContext.Provider>
    );
  }

  const isSelf = (username: string) =>
    username === session.username || username === session.email;

  return (
    <ToastContext.Provider value={toast}>
      <RouterContext.Provider value={navigate}>
        <Routes
          path={path}
          session={session}
          pool={pool}
          isSelf={isSelf}
          onSignOut={signOut}
          navigate={navigate}
          onGroupsChanged={() => void reloadPool()}
        />
        <Toasts toasts={toast.toasts} dismiss={toast.dismiss} />
      </RouterContext.Provider>
    </ToastContext.Provider>
  );
}

interface RoutesProps {
  path: string;
  session: SessionInfo;
  pool: PoolInfo;
  isSelf: (username: string) => boolean;
  onSignOut: () => void;
  navigate: (to: string, replace?: boolean) => void;
  onGroupsChanged: () => void;
}

function Routes({
  path,
  session,
  pool,
  isSelf,
  onSignOut,
  navigate,
  onGroupsChanged,
}: RoutesProps) {
  const home = session.isAdmin ? "/admin" : "/account";
  const wantsAdmin = path.startsWith("/admin");

  useEffect(() => {
    if (path === "/" || (wantsAdmin && !session.isAdmin)) navigate(home, true);
  }, [path, wantsAdmin, session.isAdmin, home, navigate]);

  const chrome = (current: "admin" | "groups" | "account", children: ReactNode) => (
    <Layout session={session} current={current} onSignOut={onSignOut}>
      {children}
    </Layout>
  );

  if (wantsAdmin && session.isAdmin) {
    if (path === "/admin/groups") {
      return chrome(
        "groups",
        <AdminGroups adminGroup={pool.adminGroup} onGroupsChanged={onGroupsChanged} />,
      );
    }
    const group = path.match(/^\/admin\/groups\/(.+)$/);
    if (group) {
      return chrome(
        "groups",
        <AdminGroupDetail
          key={group[1]}
          group={decodeURIComponent(group[1])}
          adminGroup={pool.adminGroup}
          onGroupsChanged={onGroupsChanged}
        />,
      );
    }
    if (path === "/admin/users/new") {
      return chrome(
        "admin",
        <AdminUserCreate
          fields={pool.adminVisible}
          groups={pool.groups}
          usernameIsEmail={pool.usernameIsEmail}
        />,
      );
    }
    const match = path.match(/^\/admin\/users\/(.+)$/);
    if (match) {
      return chrome(
        "admin",
        <AdminUserDetail
          key={match[1]}
          username={decodeURIComponent(match[1])}
          fields={pool.adminVisible}
          editable={pool.editable}
          groups={pool.groups}
          poolMfa={pool.mfaConfiguration}
          isSelf={isSelf}
        />,
      );
    }
    return chrome(
      "admin",
      <AdminUsers poolName={pool.name ?? pool.id} searchFields={pool.searchFields} />,
    );
  }

  return chrome(
    "account",
    <Account fields={pool.selfEditable} poolMfa={pool.mfaConfiguration} />,
  );
}
