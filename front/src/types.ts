export type DataType = "string" | "number" | "dateTime" | "boolean";

export interface AttributeField {
  name: string;
  dataType: DataType;
  mutable: boolean;
  required: boolean;
  isCustom: boolean;
  minLength: number | null;
  maxLength: number | null;
  minValue: number | null;
  maxValue: number | null;
}

export interface SessionInfo {
  username: string;
  email: string | null;
  groups: string[];
  isAdmin: boolean;
}

export interface PoolInfo {
  id: string;
  name: string | null;
  usernameIsEmail: boolean;
  selfEditable: AttributeField[];
  adminVisible: AttributeField[];
  editable: AttributeField[];
  groups: string[];
  /** Attributes the user search may filter on; empty for a non-admin. */
  searchFields: string[];
}

export interface PublicInfo {
  poolName: string | null;
  version: string;
}

export type Attributes = Record<string, string>;

/** `null` asks the server to delete the attribute. */
export type AttributePatch = Record<string, string | null>;

export interface UserSummary {
  username: string;
  enabled: boolean;
  status: string | null;
  createdAt: string | null;
  attributes: Attributes;
}

export interface UserDetail extends UserSummary {
  updatedAt: string | null;
  groups: string[];
  mfa: string[];
  preferredMfa: string | null;
}

export interface UserPage {
  users: UserSummary[];
  nextToken: string | null;
}

export interface MyProfile {
  username: string;
  attributes: Attributes;
  mfa: string[];
  preferredMfa: string | null;
  groups: string[];
}

export interface Challenge {
  name: string;
  requiredAttributes: string[];
  mfaOptions: string[];
  destination: string | null;
}

export type AuthOutcome =
  | { status: "signedIn" }
  | { status: "challenge"; challenge: Challenge };

export interface ChallengeAnswer {
  newPassword?: string;
  confirmPassword?: string;
  code?: string;
  mfaType?: string;
  userAttributes?: Record<string, string>;
}
