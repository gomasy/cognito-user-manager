import type { ReactNode } from "react";
import { useT } from "../hooks";
import type { SessionInfo } from "../types";
import { Link } from "./Link";

interface Props {
  session: SessionInfo;
  current: "admin" | "groups" | "account";
  onSignOut: () => void;
  children: ReactNode;
}

export function Layout({ session, current, onSignOut, children }: Props) {
  const t = useT();

  const link = (to: string, key: string, active: boolean) => (
    <Link to={to} className={active ? "nav__link nav__link--active" : "nav__link"}>
      {t(key)}
    </Link>
  );

  return (
    <>
      <header className="topbar">
        <div className="topbar__inner">
          <Link to="/" className="brand">
            {t("app.title")}
          </Link>
          <nav className="nav">
            {session.isAdmin && link("/admin", "nav.users", current === "admin")}
            {session.isAdmin && link("/admin/groups", "nav.groups", current === "groups")}
            {link("/account", "nav.account", current === "account")}
          </nav>
          <div className="who">
            <span>{session.email ?? session.username}</span>
            {session.isAdmin && <span className="badge badge--ok">{t("nav.admin")}</span>}
          </div>
          <button type="button" className="btn" onClick={onSignOut}>
            {t("nav.signOut")}
          </button>
        </div>
      </header>
      {children}
    </>
  );
}
