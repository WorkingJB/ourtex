// Build/parse the canonical document source (YAML frontmatter + markdown
// body) that `ourtex-server`'s vault endpoints exchange.
//
// The server re-parses and re-serializes whatever we PUT via
// `ourtex-vault::Document`, so we don't have to match the canonical byte
// form exactly — valid YAML plus a body is enough. On GET the server
// always returns canonical form; `parseSource` only needs to handle that.
import YAML from "js-yaml";

export type DocFrontmatter = {
  id: string;
  type: string;
  visibility: string;
  tags: string[];
  links: string[];
  aliases: string[];
  source: string | null;
  created: string | null;
  updated: string | null;
};

export type DocDetail = DocFrontmatter & {
  body: string;
  version: string;
  updated_at: string;
};

const OPEN = "---\n";
const CLOSE = "\n---\n";

export function parseSource(
  source: string,
  meta: { version: string; updated_at: string }
): DocDetail {
  if (!source.startsWith(OPEN)) {
    throw new Error("document source is missing opening `---` fence");
  }
  const rest = source.slice(OPEN.length);
  const end = rest.indexOf(CLOSE);
  if (end < 0) {
    throw new Error("document source is missing closing `---` fence");
  }
  const yamlText = rest.slice(0, end);
  const body = rest.slice(end + CLOSE.length);

  const fm = (YAML.load(yamlText) ?? {}) as Record<string, unknown>;
  const s = (k: string): string | null => {
    const v = fm[k];
    return typeof v === "string" ? v : null;
  };
  const arr = (k: string): string[] => {
    const v = fm[k];
    if (!Array.isArray(v)) return [];
    return v.filter((x): x is string => typeof x === "string");
  };

  const id = s("id");
  const type = s("type");
  const visibility = s("visibility");
  if (!id || !type || !visibility) {
    throw new Error("document frontmatter missing required id/type/visibility");
  }
  return {
    id,
    type,
    visibility,
    tags: arr("tags"),
    links: arr("links"),
    aliases: arr("aliases"),
    source: s("source"),
    created: s("created"),
    updated: s("updated"),
    body,
    version: meta.version,
    updated_at: meta.updated_at,
  };
}

export function buildSource(input: {
  id: string;
  type: string;
  visibility: string;
  tags?: string[];
  links?: string[];
  aliases?: string[];
  source?: string | null;
  body: string;
}): string {
  // Ordered insertion so the dumped YAML reads the same way a hand-edited
  // frontmatter would — the server re-canonicalizes regardless, but
  // keeping a predictable order makes round-tripping easier to inspect.
  const fm: Record<string, unknown> = {
    id: input.id,
    type: input.type,
    visibility: input.visibility,
  };
  if (input.tags && input.tags.length) fm.tags = input.tags;
  if (input.links && input.links.length) fm.links = input.links;
  if (input.aliases && input.aliases.length) fm.aliases = input.aliases;
  if (input.source != null && input.source !== "") fm.source = input.source;

  const yamlText = YAML.dump(fm, { lineWidth: -1, noRefs: true });
  // `yamlText` always ends with a newline; the close fence expects the
  // previous line to terminate cleanly, so don't double it.
  return `${OPEN}${yamlText}---\n${input.body}`;
}
