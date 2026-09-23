type Kind<Value>
  = [Value] extends [boolean] ? "boolean"
    : [Value] extends [number] ? "integer" | "number"
        : [Value] extends [string] ? (string extends Value ? "string" : { oneOf: readonly Value[]; })
            : [Value] extends [object] ? { shape: Shape<Value>; }
                : never;

type FieldKind<Target, Key extends keyof Target> = object extends Pick<Target, Key>
  ? { optional: Kind<Exclude<Target[Key], undefined>>; }
  : Kind<Target[Key]>;

// Every key of the generated type must be listed with the kind its type implies, so a
// regenerated schema that adds or retypes a field fails to compile here.
export type Shape<Target> = { readonly [Key in keyof Target]-?: FieldKind<Target, Key>; };

type AnyKind
  = | "boolean"
    | "integer"
    | "number"
    | "string"
    | { oneOf: readonly unknown[]; }
    | { shape: AnyShape; }
    | { optional: AnyKind; };

type AnyShape = Readonly<Record<string, AnyKind>>;

const isOfKind = (kind: AnyKind, value: unknown): boolean => {
  if (kind === "boolean") return typeof value === "boolean";

  if (kind === "integer") return Number.isSafeInteger(value);

  if (kind === "number") return Number.isFinite(value);

  if (kind === "string") return typeof value === "string";

  if ("oneOf" in kind) return kind.oneOf.includes(value);

  if ("optional" in kind) return value === undefined || isOfKind(kind.optional, value);

  return isShaped(kind.shape, value);
};

const isShaped = (shape: AnyShape, value: unknown): boolean =>
  typeof value === "object"
  && value !== null
  && !Array.isArray(value)
  && Object.entries(shape).every(([key, kind]) => isOfKind(kind, Reflect.get(value, key)));

export const hasShape = <Target>(shape: Shape<Target> & AnyShape, value: unknown): value is Target =>
  isShaped(shape, value);
