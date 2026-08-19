import { getLang, t } from "./i18n";
import type {
  AttributePatch,
  AuthOutcome,
  ChallengeAnswer,
  MyProfile,
  PoolInfo,
  PublicInfo,
  SessionInfo,
  UserDetail,
  UserPage,
} from "./types";

/** Thrown on a 401 so callers can send the user back to the sign-in screen. */
export class UnauthorizedError extends Error {}

/**
 * Fired once per 401 so the shell can drop back to sign-in without every call
 * site having to handle it.
 */
export const UNAUTHORIZED_EVENT = "cum:unauthorized";

/** Any other non-OK response; the message is already localized by the server. */
export class ApiError extends Error {}

async function errorMessage(res: Response): Promise<string> {
  try {
    const body = await res.json();
    if (body && typeof body.error === "string") return body.error;
  } catch {
    // Not a JSON error body; fall through to the generic wording.
  }
  return t("api.requestFailed");
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: {
      "Content-Type": "application/json",
      // Advertise the UI language so the server localizes its messages to match.
      "X-App-Lang": getLang(),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  if (res.status === 401) {
    const message = await errorMessage(res);
    window.dispatchEvent(new CustomEvent(UNAUTHORIZED_EVENT));
    throw new UnauthorizedError(message);
  }
  if (!res.ok) throw new ApiError(await errorMessage(res));
  if (res.status === 204) return undefined as T;
  return res.json();
}

/** Routed through here so no call site can forget to escape a username. */
const userPath = (username: string) =>
  `/api/admin/users/${encodeURIComponent(username)}`;

interface MessageResponse {
  message: string;
}

export const api = {
  publicInfo: () => request<PublicInfo>("GET", "/api/public"),
  session: () => request<SessionInfo>("GET", "/api/session"),
  pool: () => request<PoolInfo>("GET", "/api/pool"),

  login: (username: string, password: string) =>
    request<AuthOutcome>("POST", "/api/auth/login", { username, password }),
  answerChallenge: (answer: ChallengeAnswer) =>
    request<AuthOutcome>("POST", "/api/auth/challenge", answer),
  logout: () => request<void>("POST", "/api/auth/logout"),

  profile: () => request<MyProfile>("GET", "/api/account"),
  updateProfile: (attributes: AttributePatch) =>
    request<MessageResponse>("PATCH", "/api/account", { attributes }),
  changePassword: (
    currentPassword: string,
    newPassword: string,
    confirmPassword: string,
  ) =>
    request<MessageResponse>("POST", "/api/account/password", {
      currentPassword,
      newPassword,
      confirmPassword,
    }),
  sendVerificationCode: (attribute: string) =>
    request<MessageResponse>("POST", "/api/account/verify/send", { attribute }),
  verifyAttribute: (attribute: string, code: string) =>
    request<MessageResponse>("POST", "/api/account/verify", { attribute, code }),

  listUsers: (query: { q?: string; field?: string; token?: string }) => {
    const params = new URLSearchParams();
    if (query.q) params.set("q", query.q);
    if (query.field) params.set("field", query.field);
    if (query.token) params.set("token", query.token);
    const search = params.toString();
    return request<UserPage>("GET", `/api/admin/users${search ? `?${search}` : ""}`);
  },
  createUser: (payload: {
    username: string;
    attributes: AttributePatch;
    temporaryPassword: string;
    suppressMessage: boolean;
    groups: string[];
  }) => request<MessageResponse & { username: string }>("POST", "/api/admin/users", payload),
  user: (username: string) => request<UserDetail>("GET", userPath(username)),
  updateUser: (username: string, attributes: AttributePatch) =>
    request<MessageResponse>("PATCH", userPath(username), { attributes }),
  deleteUser: (username: string) =>
    request<MessageResponse>("DELETE", userPath(username)),
  setGroups: (username: string, groups: string[]) =>
    request<MessageResponse>("PUT", `${userPath(username)}/groups`, { groups }),
  setPassword: (username: string, password: string, permanent: boolean) =>
    request<MessageResponse>("POST", `${userPath(username)}/password`, {
      password,
      permanent,
    }),
  resetPassword: (username: string) =>
    request<MessageResponse>("POST", `${userPath(username)}/password/reset`),
  setEnabled: (username: string, enabled: boolean) =>
    request<MessageResponse>("POST", `${userPath(username)}/enabled`, { enabled }),
  signOutUser: (username: string) =>
    request<MessageResponse>("POST", `${userPath(username)}/signout`),
  resendInvite: (username: string) =>
    request<MessageResponse>("POST", `${userPath(username)}/invite`),
};
