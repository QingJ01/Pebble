import { useEditorState } from "@tiptap/react";
import {
  Bold, Italic, Strikethrough, Heading1, Heading2,
  List, ListOrdered, Quote, Code, Minus, Undo2, Redo2,
  Link, Image,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Editor } from "@tiptap/react";

// ─── Mode Button ───────────────────────────────────────────────────────────────

export function ModeButton({ icon: Icon, label, active, onClick }: {
  icon: React.ElementType; label: string; active: boolean; onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      className={`compose-mode-button${active ? " is-active" : ""}`}
    >
      <Icon size={13} />
      {label}
    </button>
  );
}

// ─── Editor Toolbar ────────────────────────────────────────────────────────────

export function EditorToolbar({ editor }: { editor: Editor }) {
  const { t } = useTranslation();

  const activeStates = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      strike: e.isActive("strike"),
      h1: e.isActive("heading", { level: 1 }),
      h2: e.isActive("heading", { level: 2 }),
      bulletList: e.isActive("bulletList"),
      orderedList: e.isActive("orderedList"),
      blockquote: e.isActive("blockquote"),
      codeBlock: e.isActive("codeBlock"),
    }),
  });

  function btn(icon: React.ElementType, label: string, action: () => void, active?: boolean) {
    return { icon, label, action, active };
  }

  const items = [
    btn(Bold, t("compose.toolbar.bold", "Bold"), () => editor.chain().focus().toggleBold().run(), activeStates.bold),
    btn(Italic, t("compose.toolbar.italic", "Italic"), () => editor.chain().focus().toggleItalic().run(), activeStates.italic),
    btn(Strikethrough, t("compose.toolbar.strike", "Strikethrough"), () => editor.chain().focus().toggleStrike().run(), activeStates.strike),
    btn(Heading1, t("compose.toolbar.heading1"), () => editor.chain().focus().toggleHeading({ level: 1 }).run(), activeStates.h1),
    btn(Heading2, t("compose.toolbar.heading2"), () => editor.chain().focus().toggleHeading({ level: 2 }).run(), activeStates.h2),
    btn(List, t("compose.toolbar.bulletList", "Bullet list"), () => editor.chain().focus().toggleBulletList().run(), activeStates.bulletList),
    btn(ListOrdered, t("compose.toolbar.orderedList", "Ordered list"), () => editor.chain().focus().toggleOrderedList().run(), activeStates.orderedList),
    btn(Quote, t("compose.toolbar.blockquote", "Quote"), () => editor.chain().focus().toggleBlockquote().run(), activeStates.blockquote),
    btn(Code, t("compose.toolbar.code", "Code"), () => editor.chain().focus().toggleCodeBlock().run(), activeStates.codeBlock),
    btn(Minus, t("compose.toolbar.hr", "Divider"), () => editor.chain().focus().setHorizontalRule().run()),
    btn(Undo2, t("compose.toolbar.undo", "Undo"), () => editor.chain().focus().undo().run()),
    btn(Redo2, t("compose.toolbar.redo", "Redo"), () => editor.chain().focus().redo().run()),
  ];

  return (
    <div className="compose-format-toolbar">
      {items.map((item, i) => {
        const Icon = item.icon;
        return (
          <button
            key={i}
            type="button"
            onClick={item.action}
            title={item.label}
            aria-label={item.label}
            className={`compose-format-button${item.active ? " is-active" : ""}`}
          >
            <Icon size={15} />
          </button>
        );
      })}
    </div>
  );
}

// ─── Markdown Toolbar ─────────────────────────────────────────────────────────

export function MarkdownToolbar({ textareaRef, onInsert, source }: {
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  onInsert: (value: string) => void;
  source: string;
}) {
  const { t } = useTranslation();

  function insert(before: string, after = "", placeholder = "") {
    const ta = textareaRef.current;
    if (!ta) {
      onInsert(source + before + placeholder + after);
      return;
    }
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const selected = source.slice(start, end) || placeholder;
    const newText = source.slice(0, start) + before + selected + after + source.slice(end);
    onInsert(newText);
    // Restore cursor position after React re-render
    requestAnimationFrame(() => {
      ta.focus();
      const cursorPos = start + before.length + selected.length;
      ta.setSelectionRange(cursorPos, cursorPos);
    });
  }

  const items = [
    { icon: Bold, label: t("compose.toolbar.bold", "Bold"), action: () => insert("**", "**", "bold") },
    { icon: Italic, label: t("compose.toolbar.italic", "Italic"), action: () => insert("*", "*", "italic") },
    { icon: Strikethrough, label: t("compose.toolbar.strike", "Strikethrough"), action: () => insert("~~", "~~", "text") },
    { icon: Heading1, label: t("compose.toolbar.heading1"), action: () => insert("\n# ", "\n", "heading") },
    { icon: Heading2, label: t("compose.toolbar.heading2"), action: () => insert("\n## ", "\n", "heading") },
    { icon: List, label: t("compose.toolbar.bulletList", "Bullet list"), action: () => insert("\n- ", "") },
    { icon: ListOrdered, label: t("compose.toolbar.orderedList", "Ordered list"), action: () => insert("\n1. ", "") },
    { icon: Quote, label: t("compose.toolbar.blockquote", "Quote"), action: () => insert("\n> ", "") },
    { icon: Code, label: t("compose.toolbar.code", "Code"), action: () => insert("`", "`", "code") },
    { icon: Minus, label: t("compose.toolbar.hr", "Divider"), action: () => insert("\n---\n", "") },
    { icon: Link, label: t("compose.toolbar.link", "Link"), action: () => insert("[", "](url)", "text") },
    { icon: Image, label: t("compose.toolbar.image", "Image"), action: () => insert("![", "](url)", "alt") },
  ];

  return (
    <div className="compose-format-toolbar">
      {items.map((item, i) => {
        const Icon = item.icon;
        return (
          <button
            key={i}
            type="button"
            onClick={item.action}
            title={item.label}
            aria-label={item.label}
            className="compose-format-button"
          >
            <Icon size={15} />
          </button>
        );
      })}
    </div>
  );
}

// ─── Shared Styles ─────────────────────────────────────────────────────────────

export const composeStyles = {
  fieldLabel: {
    padding: "8px 0", fontSize: "13px", color: "var(--color-text-secondary)",
    width: "52px", flexShrink: 0, textAlign: "right", marginRight: "8px",
  } as React.CSSProperties,

  fieldRow: {
    display: "flex", alignItems: "center",
    borderBottom: "1px solid var(--color-border)",
  } as React.CSSProperties,

  toggleBtn: {
    padding: "4px 8px", border: "none", background: "none", cursor: "pointer",
    color: "var(--color-text-secondary)", fontSize: "12px", whiteSpace: "nowrap",
    borderRadius: "4px",
  } as React.CSSProperties,

  backBtn: {
    display: "flex", alignItems: "center", gap: "4px",
    background: "none", border: "none", cursor: "pointer",
    color: "var(--color-text-secondary)", fontSize: "13px",
    padding: "4px 8px", borderRadius: "4px",
  } as React.CSSProperties,
};
