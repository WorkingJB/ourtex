import { useCallback, useEffect, useState } from "react";
import { useEditor, EditorContent, Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import { Markdown } from "tiptap-markdown";

/// Markdown-backed rich text editor used in DocEditor's Content
/// field. The user works in WYSIWYG by default (toolbar palette);
/// flipping the Advanced toggle exposes the raw markdown so power
/// users can edit MD directly. Both modes round-trip through the
/// same `value` prop, which is plain markdown.
export function RichTextEditor({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (markdown: string) => void;
  placeholder?: string;
}) {
  const [advanced, setAdvanced] = useState(false);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // Tiptap-markdown handles its own MD round-tripping; the
        // StarterKit's default link extension gets in its way, so
        // we replace it with our own.
        link: false,
        // We don't expose horizontal rules through the toolbar; let
        // them through if the user types `---` though.
      }),
      Link.configure({
        openOnClick: false,
        autolink: true,
        protocols: ["http", "https", "mailto"],
        HTMLAttributes: {
          class: "underline text-brand-700",
          rel: "noopener noreferrer",
        },
      }),
      Markdown.configure({
        html: false,
        breaks: true,
        transformPastedText: true,
      }),
    ],
    content: value,
    onUpdate: ({ editor }) => {
      const md = (editor.storage as any).markdown?.getMarkdown?.() ?? "";
      onChange(md);
    },
    editorProps: {
      attributes: {
        class:
          "prose prose-sm max-w-none focus:outline-none min-h-[20rem] " +
          "[&_p]:my-2 [&_h1]:text-xl [&_h1]:font-semibold [&_h1]:mt-4 " +
          "[&_h2]:text-lg [&_h2]:font-semibold [&_h2]:mt-3 " +
          "[&_h3]:text-base [&_h3]:font-semibold [&_h3]:mt-3 " +
          "[&_ul]:list-disc [&_ul]:pl-6 [&_ol]:list-decimal [&_ol]:pl-6 " +
          "[&_code]:bg-neutral-100 [&_code]:px-1 [&_code]:rounded " +
          "[&_blockquote]:border-l-2 [&_blockquote]:border-neutral-300 " +
          "[&_blockquote]:pl-3 [&_blockquote]:text-neutral-600",
      },
    },
  });

  // External value changes (e.g. switching docs) re-seed the editor.
  // We intentionally compare against the current MD output to avoid
  // resetting on every keystroke when our own onUpdate echoes back.
  useEffect(() => {
    if (!editor) return;
    const current = (editor.storage as any).markdown?.getMarkdown?.() ?? "";
    if (current !== value) {
      editor.commands.setContent(value, { emitUpdate: false } as any);
    }
  }, [editor, value]);

  return (
    <div className="border border-neutral-300 rounded">
      <Toolbar editor={editor} advanced={advanced} onToggleAdvanced={() => setAdvanced((v) => !v)} />
      {advanced ? (
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          rows={20}
          placeholder={placeholder}
          className="w-full px-3 py-2 text-sm font-mono leading-relaxed border-0 focus:outline-none focus:ring-0 resize-vertical"
        />
      ) : (
        <div className="px-3 py-2 min-h-[20rem]">
          <EditorContent editor={editor} />
        </div>
      )}
    </div>
  );
}

function Toolbar({
  editor,
  advanced,
  onToggleAdvanced,
}: {
  editor: Editor | null;
  advanced: boolean;
  onToggleAdvanced: () => void;
}) {
  const can = useCallback(
    (fn: (chain: ReturnType<NonNullable<typeof editor>["chain"]>) => unknown) =>
      editor ? Boolean(fn(editor.chain().focus())) : false,
    [editor]
  );

  return (
    <div className="flex items-center gap-1 px-2 py-1 border-b border-neutral-200 bg-neutral-50 text-sm">
      <ToolbarBtn
        label="B"
        title="Bold (⌘B)"
        active={editor?.isActive("bold") ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleBold().run())}
        className="font-bold"
      />
      <ToolbarBtn
        label="I"
        title="Italic (⌘I)"
        active={editor?.isActive("italic") ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleItalic().run())}
        className="italic"
      />
      <Sep />
      <ToolbarBtn
        label="H2"
        title="Heading 2"
        active={editor?.isActive("heading", { level: 2 }) ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleHeading({ level: 2 }).run())}
      />
      <ToolbarBtn
        label="H3"
        title="Heading 3"
        active={editor?.isActive("heading", { level: 3 }) ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleHeading({ level: 3 }).run())}
      />
      <Sep />
      <ToolbarBtn
        label="•"
        title="Bullet list"
        active={editor?.isActive("bulletList") ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleBulletList().run())}
      />
      <ToolbarBtn
        label="1."
        title="Numbered list"
        active={editor?.isActive("orderedList") ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleOrderedList().run())}
      />
      <Sep />
      <ToolbarBtn
        label="❝"
        title="Quote"
        active={editor?.isActive("blockquote") ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleBlockquote().run())}
      />
      <ToolbarBtn
        label="‹›"
        title="Inline code"
        active={editor?.isActive("code") ?? false}
        disabled={advanced || !editor}
        onClick={() => can((c) => c.toggleCode().run())}
      />
      <ToolbarBtn
        label="🔗"
        title="Link"
        active={editor?.isActive("link") ?? false}
        disabled={advanced || !editor}
        onClick={() => {
          if (!editor) return;
          const href = window.prompt(
            "URL (leave blank to remove the link):",
            editor.getAttributes("link").href ?? ""
          );
          if (href === null) return;
          if (href === "") {
            editor.chain().focus().unsetLink().run();
          } else {
            editor.chain().focus().setLink({ href }).run();
          }
        }}
      />
      <div className="ml-auto">
        <button
          onClick={onToggleAdvanced}
          title="Advanced — show raw markdown"
          className={
            "text-xs px-2 py-1 rounded transition " +
            (advanced
              ? "bg-brand-100 text-brand-700 font-medium"
              : "text-neutral-500 hover:bg-neutral-100")
          }
        >
          {advanced ? "Rich" : "Advanced"}
        </button>
      </div>
    </div>
  );
}

function Sep() {
  return <div className="w-px h-5 bg-neutral-200 mx-1" />;
}

function ToolbarBtn({
  label,
  title,
  active,
  disabled,
  onClick,
  className,
}: {
  label: string;
  title: string;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      // Drop synthetic detail:0 clicks — see the matching fix in
      // apps/web/src/RichTextEditor.tsx for the full diagnosis. Real
      // mouse clicks always have detail >= 1.
      onClick={(e) => {
        if (e.detail === 0) return;
        onClick();
      }}
      title={title}
      disabled={disabled}
      className={[
        "min-w-[1.75rem] h-7 px-1.5 rounded text-xs transition",
        active
          ? "bg-brand-100 text-brand-700"
          : "text-neutral-700 hover:bg-neutral-100",
        disabled ? "opacity-50 cursor-not-allowed" : "",
        className ?? "",
      ].join(" ")}
    >
      {label}
    </button>
  );
}
