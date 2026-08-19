import type { ReactNode } from "react";
import { useNavigate, useT } from "../hooks";
import type { SessionInfo } from "../types";
import { LanguagePicker } from "./Login";

interface Props {
  session: SessionInfo;
  current: "admin" | "account";
  onSignOut: () => void;
  children: ReactNode;
}

export function Layout({ session, current, onSignOut, children }: Props) {
  const t = useT();
  const navigate = useNavigate();

  const link = (to: string, key: string, active: boolean) => (
    <a
      href={to}
      className={active ? "nav__link nav__link--active" : "nav__link"}
      onClick={(event) => {
        event.preventDefault();
        navigate(to);
      }}
    >
      {t(key)}
    </a>
  );

  return (
    <>
      <header className="topbar">
        <div className="topbar__inner">
          <a
            href="/"
            className="brand"
            onClick={(event) => {
              event.preventDefault();
              navigate("/");
            }}
          >
            {t("app.title")}
          </a>
          <nav className="nav">
            {session.isAdmin && link("/admin", "nav.users", current === "admin")}
            {link("/account", "nav.account", current === "account")}
          </nav>
          <div className="who">
            <span>{session.email ?? session.username}</span>
            {session.isAdmin && <span className="badge badge--ok">{t("nav.admin")}</span>}
          </div>
          <LanguagePicker />
          <button type="button" className="btn" onClick={onSignOut}>
            {t("nav.signOut")}
          </button>
        </div>
      </header>
      {children}
    </>
  );
}
