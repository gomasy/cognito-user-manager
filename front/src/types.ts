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
  /** "OFF", "ON" or "OPTIONAL"; a pool with MFA off rejects every preference. */
  mfaConfiguration: string;
  selfEditable: AttributeField[];
  adminVisible: AttributeField[];
  editable: AttributeField[];
  groups: string[];
  /** The group that grants admin access; null for a non-admin. */
  adminGroup: string | null;
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

export interface GroupInfo {
  name: string;
  description: string | null;
  /** Lower wins when a user is in several groups. */
  precedence: number | null;
  roleArn: string | null;
  createdAt: string | null;
  updatedAt: string | null;
}

/**
 * Factors to switch on or off. A factor left out is not sent to Cognito at
 * all, which is what keeps a pool that lacks it from rejecting the request.
 */
export interface MfaPreference {
  sms?: boolean;
  softwareToken?: boolean;
  email?: boolean;
  preferred?: string | null;
}

export interface TotpSetup {
  secretCode: string;
  otpauthUri: string;
  /** An SVG data URI, or null if the URI was too long to encode. */
  qrDataUri: string | null;
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
