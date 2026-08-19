import { useLabel, useT } from "../hooks";
import type { AttributeField, Attributes, AttributePatch } from "../types";

const INPUT_TYPES: Record<string, string> = {
  email: "email",
  phone_number: "tel",
  birthdate: "date",
  website: "url",
  profile: "url",
  picture: "url",
};

function inputType(field: AttributeField): string {
  if (field.dataType === "number") return "number";
  return INPUT_TYPES[field.name] ?? "text";
}

/** Draft values keyed by attribute name; booleans are "true" / "false". */
export type Draft = Attributes;

export function initialDraft(fields: AttributeField[], values: Attributes): Draft {
  const draft: Draft = {};
  for (const field of fields) draft[field.name] = values[field.name] ?? "";
  return draft;
}

/**
 * Turns a draft into the patch the API expects: an empty value asks for the
 * attribute to be deleted. Only the fields rendered here are named, so a screen
 * showing a subset cannot clear the rest.
 */
export function toPatch(fields: AttributeField[], draft: Draft): AttributePatch {
  const patch: AttributePatch = {};
  for (const field of fields) {
    const value = (draft[field.name] ?? "").trim();
    patch[field.name] = value === "" ? null : value;
  }
  return patch;
}

interface Props {
  fields: AttributeField[];
  draft: Draft;
  onChange: (name: string, value: string) => void;
  /** Create screens may set immutable attributes; edit screens may not. */
  allowImmutable?: boolean;
}

export function AttributeFields({ fields, draft, onChange, allowImmutable }: Props) {
  const t = useT();
  const label = useLabel();

  return (
    <div className="grid-2">
      {fields.map((field) => {
        const disabled = !allowImmutable && !field.mutable;
        const value = draft[field.name] ?? "";

        if (field.dataType === "boolean") {
          return (
            <div className="field" key={field.name}>
              <span className="field__label">{label(field.name)}</span>
              <label className="check">
                <input
                  type="checkbox"
                  checked={value === "true"}
                  disabled={disabled}
                  onChange={(event) =>
                    onChange(field.name, event.target.checked ? "true" : "false")
                  }
                />
                <span>{t("common.yes")}</span>
              </label>
            </div>
          );
        }

        return (
          <label className="field" key={field.name}>
            <span className="field__label">
              {label(field.name)}
              {field.required && <span className="field__required">*</span>}
              {field.isCustom && <span className="tag">{t("attr.custom")}</span>}
              {disabled && <span className="tag">{t("attr.readOnly")}</span>}
            </span>
            <input
              type={inputType(field)}
              value={value}
              disabled={disabled}
              required={field.required}
              minLength={field.minLength ?? undefined}
              maxLength={field.maxLength ?? undefined}
              min={field.minValue ?? undefined}
              max={field.maxValue ?? undefined}
              placeholder={field.name === "phone_number" ? "+819012345678" : undefined}
              autoComplete="off"
              onChange={(event) => onChange(field.name, event.target.value)}
            />
            {field.name === "phone_number" && (
              <span className="hint">{t("attr.phoneHint")}</span>
            )}
          </label>
        );
      })}
    </div>
  );
}
