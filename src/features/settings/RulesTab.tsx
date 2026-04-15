import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Pencil, Trash2, ShieldCheck, X } from "lucide-react";
import { createRule, deleteRule, listRules, updateRule, type Rule } from "@/lib/api";
import ConfirmDialog from "@/components/ConfirmDialog";
import { useToastStore } from "@/stores/toast.store";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import {
  parseRuleActions,
  parseRuleConditions,
  serializeRuleActions,
  serializeRuleConditions,
} from "./rule-json";

// ── Types for visual builder ────────────────────────────────────
type ConditionField = "from" | "to" | "subject" | "body" | "has_attachment" | "domain";
type ConditionOp = "contains" | "not_contains" | "equals" | "starts_with" | "ends_with";

interface Condition {
  field: ConditionField;
  op: ConditionOp;
  value: string;
}

type ActionType = "AddLabel" | "MoveToFolder" | "MarkRead" | "Archive" | "SetKanbanColumn";
type KanbanColumn = "todo" | "waiting" | "done";

interface RuleAction {
  type: ActionType;
  value?: string;
}

const CONDITION_FIELDS: ConditionField[] = ["from", "to", "subject", "body", "has_attachment", "domain"];
const CONDITION_OPS: ConditionOp[] = ["contains", "not_contains", "equals", "starts_with", "ends_with"];
const ACTION_TYPES: ActionType[] = ["AddLabel", "MoveToFolder", "MarkRead", "Archive", "SetKanbanColumn"];
const KANBAN_COLUMNS: KanbanColumn[] = ["todo", "waiting", "done"];

// ── Helpers to convert between visual model and JSON string ─────
function parseConditions(json: string): Condition[] {
  return parseRuleConditions(json);
}

function serializeConditions(conditions: Condition[]): string {
  return serializeRuleConditions(conditions);
}

function parseActions(json: string): RuleAction[] {
  return parseRuleActions(json);
}

function serializeActions(actions: RuleAction[]): string {
  return serializeRuleActions(actions);
}

// ── Form state ──────────────────────────────────────────────────
interface RuleFormData {
  name: string;
  priority: number;
  is_enabled: boolean;
  conditions: Condition[];
  actions: RuleAction[];
}

const emptyForm: RuleFormData = {
  name: "",
  priority: 0,
  is_enabled: true,
  conditions: [{ field: "from", op: "contains", value: "" }],
  actions: [{ type: "Archive" }],
};

export default function RulesTab() {
  const { t } = useTranslation();
  const addToast = useToastStore((s) => s.addToast);
  const [rules, setRules] = useState<Rule[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [form, setForm] = useState<RuleFormData>(emptyForm);
  const [error, setError] = useState<string | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<{ id: string; name: string } | null>(null);

  async function fetchRules() {
    setFetchError(null);
    try {
      const result = await listRules();
      setRules(result);
    } catch (err) {
      console.error("Failed to fetch rules:", err);
      setFetchError(extractErrorMessage(err));
    }
  }

  useEffect(() => {
    fetchRules();
  }, []);

  function startCreate() {
    setEditingId("__new__");
    setForm(emptyForm);
    setError(null);
  }

  function startEdit(rule: Rule) {
    setEditingId(rule.id);
    setForm({
      name: rule.name,
      priority: rule.priority,
      is_enabled: rule.is_enabled,
      conditions: parseConditions(rule.conditions),
      actions: parseActions(rule.actions),
    });
    setError(null);
  }

  function cancelEdit() {
    setEditingId(null);
    setForm(emptyForm);
    setError(null);
  }

  async function handleSave() {
    if (!form.name.trim()) {
      setError(t("rules.nameRequired"));
      return;
    }

    const conditionsJson = serializeConditions(form.conditions);
    const actionsJson = serializeActions(form.actions);

    try {
      if (editingId === "__new__") {
        await createRule(form.name, form.priority, conditionsJson, actionsJson);
      } else if (editingId) {
        const existing = rules.find((r) => r.id === editingId);
        if (existing) {
          await updateRule({
            ...existing,
            name: form.name,
            priority: form.priority,
            is_enabled: form.is_enabled,
            conditions: conditionsJson,
            actions: actionsJson,
          });
        }
      }
      cancelEdit();
      await fetchRules();
    } catch (err) {
      setError(String(err));
    }
  }

  async function doDelete(ruleId: string) {
    try {
      await deleteRule(ruleId);
      if (editingId === ruleId) cancelEdit();
      await fetchRules();
    } catch (err) {
      addToast({ message: t("rules.deleteFailed", "Failed to delete rule"), type: "error" });
      console.error("Failed to delete rule:", err);
    }
  }

  // ── Condition row helpers ───────────────────────────────────
  function updateCondition(index: number, patch: Partial<Condition>) {
    setForm((f) => ({
      ...f,
      conditions: f.conditions.map((c, i) => (i === index ? { ...c, ...patch } : c)),
    }));
  }

  function addCondition() {
    setForm((f) => ({
      ...f,
      conditions: [...f.conditions, { field: "from", op: "contains", value: "" }],
    }));
  }

  function removeCondition(index: number) {
    setForm((f) => ({
      ...f,
      conditions: f.conditions.filter((_, i) => i !== index),
    }));
  }

  // ── Action row helpers ──────────────────────────────────────
  function updateAction(index: number, patch: Partial<RuleAction>) {
    setForm((f) => ({
      ...f,
      actions: f.actions.map((a, i) => (i === index ? { ...a, ...patch } : a)),
    }));
  }

  function addAction() {
    setForm((f) => ({
      ...f,
      actions: [...f.actions, { type: "Archive" }],
    }));
  }

  function removeAction(index: number) {
    setForm((f) => ({
      ...f,
      actions: f.actions.filter((_, i) => i !== index),
    }));
  }

  // ── Styles ──────────────────────────────────────────────────
  const inputStyle: React.CSSProperties = {
    width: "100%",
    padding: "8px 10px",
    borderRadius: "6px",
    border: "1px solid var(--color-border)",
    backgroundColor: "var(--color-bg)",
    color: "var(--color-text-primary)",
    fontSize: "13px",
    boxSizing: "border-box",
  };

  const selectStyle: React.CSSProperties = {
    ...inputStyle,
    appearance: "auto",
    cursor: "pointer",
  };

  const labelStyle: React.CSSProperties = {
    display: "block",
    fontSize: "12px",
    fontWeight: 500,
    color: "var(--color-text-secondary)",
    marginBottom: "4px",
  };

  const smallBtnStyle: React.CSSProperties = {
    padding: "4px",
    border: "none",
    background: "transparent",
    borderRadius: "4px",
    cursor: "pointer",
    display: "flex",
    alignItems: "center",
    color: "var(--color-text-secondary)",
    flexShrink: 0,
  };

  // ── Condition row renderer ──────────────────────────────────
  function renderConditionRow(condition: Condition, index: number) {
    return (
      <div key={index} style={{ display: "flex", gap: "6px", alignItems: "center" }}>
        <select
          value={condition.field}
          onChange={(e) => updateCondition(index, { field: e.target.value as ConditionField })}
          style={{ ...selectStyle, flex: "0 0 120px" }}
        >
          {CONDITION_FIELDS.map((f) => (
            <option key={f} value={f}>{t(`rules.field_${f}`, f)}</option>
          ))}
        </select>
        <select
          value={condition.op}
          onChange={(e) => updateCondition(index, { op: e.target.value as ConditionOp })}
          style={{ ...selectStyle, flex: "0 0 130px" }}
        >
          {CONDITION_OPS.map((op) => (
            <option key={op} value={op}>{t(`rules.op_${op}`, op)}</option>
          ))}
        </select>
        <input
          type="text"
          value={condition.value}
          onChange={(e) => updateCondition(index, { value: e.target.value })}
          placeholder={t("rules.valuePlaceholder", "Value")}
          style={{ ...inputStyle, flex: 1 }}
        />
        <button
          onClick={() => removeCondition(index)}
          style={smallBtnStyle}
          title={t("common.remove", "Remove")}
          disabled={form.conditions.length <= 1}
        >
          <X size={14} />
        </button>
      </div>
    );
  }

  // ── Action row renderer ─────────────────────────────────────
  function renderActionRow(action: RuleAction, index: number) {
    const needsValue = action.type === "AddLabel" || action.type === "MoveToFolder" || action.type === "SetKanbanColumn";

    return (
      <div key={index} style={{ display: "flex", gap: "6px", alignItems: "center" }}>
        <select
          value={action.type}
          onChange={(e) => {
            const newType = e.target.value as ActionType;
            const val = newType === "SetKanbanColumn" ? "todo" : "";
            updateAction(index, { type: newType, value: val });
          }}
          style={{ ...selectStyle, flex: "0 0 160px" }}
        >
          {ACTION_TYPES.map((at) => (
            <option key={at} value={at}>{t(`rules.action_${at}`, at)}</option>
          ))}
        </select>
        {needsValue && (
          action.type === "SetKanbanColumn" ? (
            <select
              value={action.value || "todo"}
              onChange={(e) => updateAction(index, { value: e.target.value })}
              style={{ ...selectStyle, flex: 1 }}
            >
              {KANBAN_COLUMNS.map((c) => (
                <option key={c} value={c}>{t(`rules.kanban_${c}`, c)}</option>
              ))}
            </select>
          ) : (
            <input
              type="text"
              value={action.value || ""}
              onChange={(e) => updateAction(index, { value: e.target.value })}
              placeholder={
                action.type === "AddLabel"
                  ? t("rules.labelPlaceholder", "Label name")
                  : t("rules.folderPlaceholder", "Folder name")
              }
              style={{ ...inputStyle, flex: 1 }}
            />
          )
        )}
        {!needsValue && <div style={{ flex: 1 }} />}
        <button
          onClick={() => removeAction(index)}
          style={smallBtnStyle}
          title={t("common.remove", "Remove")}
          disabled={form.actions.length <= 1}
        >
          <X size={14} />
        </button>
      </div>
    );
  }

  // ── Editor panel ────────────────────────────────────────────
  function renderEditor() {
    return (
      <div
        style={{
          padding: "16px",
          borderRadius: "8px",
          border: "1px solid var(--color-border)",
          backgroundColor: "var(--color-bg)",
          marginBottom: "16px",
          display: "flex",
          flexDirection: "column",
          gap: "14px",
        }}
      >
        {/* Name */}
        <div>
          <label style={labelStyle}>{t("rules.name")}</label>
          <input
            type="text"
            value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            placeholder={t("rules.namePlaceholder")}
            style={inputStyle}
          />
        </div>

        {/* Priority + Enabled row */}
        <div style={{ display: "flex", gap: "16px", alignItems: "flex-end" }}>
          <div style={{ flex: 1 }}>
            <label style={labelStyle}>{t("rules.priority")}</label>
            <input
              type="number"
              value={form.priority}
              onChange={(e) => setForm({ ...form, priority: Number(e.target.value) })}
              style={inputStyle}
            />
          </div>
          <div style={{ flex: 1 }}>
            <label style={labelStyle}>{t("rules.enabled")}</label>
            <button
              onClick={() => setForm({ ...form, is_enabled: !form.is_enabled })}
              style={{
                width: "100%",
                padding: "8px 10px",
                borderRadius: "6px",
                border: "1px solid var(--color-border)",
                backgroundColor: form.is_enabled ? "var(--color-accent)" : "var(--color-bg)",
                color: form.is_enabled ? "#fff" : "var(--color-text-secondary)",
                fontSize: "13px",
                cursor: "pointer",
                fontWeight: 500,
              }}
            >
              {form.is_enabled ? t("rules.enabled") : t("rules.disabled")}
            </button>
          </div>
        </div>

        {/* Conditions — visual builder */}
        <div>
          <label style={labelStyle}>{t("rules.conditions", "Conditions")}</label>
          <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
            {form.conditions.map((c, i) => renderConditionRow(c, i))}
          </div>
          <button
            onClick={addCondition}
            style={{
              marginTop: "6px",
              padding: "4px 10px",
              fontSize: "12px",
              border: "1px solid var(--color-border)",
              borderRadius: "4px",
              background: "transparent",
              color: "var(--color-text-secondary)",
              cursor: "pointer",
            }}
          >
            + {t("rules.addCondition", "Add condition")}
          </button>
        </div>

        {/* Actions — visual builder */}
        <div>
          <label style={labelStyle}>{t("rules.actions", "Actions")}</label>
          <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
            {form.actions.map((a, i) => renderActionRow(a, i))}
          </div>
          <button
            onClick={addAction}
            style={{
              marginTop: "6px",
              padding: "4px 10px",
              fontSize: "12px",
              border: "1px solid var(--color-border)",
              borderRadius: "4px",
              background: "transparent",
              color: "var(--color-text-secondary)",
              cursor: "pointer",
            }}
          >
            + {t("rules.addAction", "Add action")}
          </button>
        </div>

        {/* Error */}
        {error && (
          <p style={{ margin: 0, fontSize: "12px", color: "#ef4444" }}>{error}</p>
        )}

        {/* Buttons */}
        <div style={{ display: "flex", gap: "8px", justifyContent: "flex-end" }}>
          <button
            onClick={cancelEdit}
            style={{
              padding: "7px 14px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: "transparent",
              color: "var(--color-text-primary)",
              fontSize: "13px",
              cursor: "pointer",
            }}
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={handleSave}
            style={{
              padding: "7px 14px",
              borderRadius: "6px",
              border: "none",
              backgroundColor: "var(--color-accent)",
              color: "#fff",
              fontSize: "13px",
              fontWeight: 600,
              cursor: "pointer",
            }}
          >
            {t("common.save")}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div>
      {/* Section header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: "20px",
        }}
      >
        <h2
          style={{
            margin: 0,
            fontSize: "16px",
            fontWeight: 600,
            color: "var(--color-text-primary)",
          }}
        >
          {t("rules.title")}
        </h2>
        <button
          onClick={startCreate}
          disabled={editingId !== null}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "6px",
            padding: "7px 14px",
            borderRadius: "6px",
            border: "none",
            backgroundColor: editingId !== null ? "var(--color-border)" : "var(--color-accent)",
            color: "#fff",
            fontSize: "13px",
            fontWeight: 600,
            cursor: editingId !== null ? "not-allowed" : "pointer",
          }}
        >
          <Plus size={14} />
          {t("rules.addRule")}
        </button>
      </div>

      {/* Inline editor for new rule */}
      {editingId === "__new__" && renderEditor()}

      {/* Fetch error state */}
      {fetchError && editingId !== "__new__" && (
        <div
          className="fade-in"
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "12px",
            padding: "48px 0",
            color: "var(--color-text-secondary)",
          }}
        >
          <p style={{ color: "var(--color-error, #e53e3e)", fontSize: "14px", margin: 0 }}>
            {t("rules.fetchError", "Failed to load rules")}
          </p>
          <p style={{ fontSize: "13px", margin: 0 }}>{fetchError}</p>
          <button
            onClick={fetchRules}
            style={{
              marginTop: "4px",
              padding: "6px 16px",
              borderRadius: "4px",
              border: "1px solid var(--color-border)",
              backgroundColor: "transparent",
              color: "var(--color-accent)",
              fontSize: "13px",
              cursor: "pointer",
            }}
          >
            {t("common.retry", "Retry")}
          </button>
        </div>
      )}

      {/* Empty state */}
      {rules.length === 0 && !fetchError && editingId !== "__new__" && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "12px",
            padding: "48px 0",
            color: "var(--color-text-secondary)",
          }}
        >
          <ShieldCheck size={40} strokeWidth={1.5} />
          <p style={{ margin: 0, fontSize: "14px" }}>{t("rules.noRules")}</p>
          <button
            onClick={startCreate}
            style={{
              marginTop: "4px",
              padding: "8px 18px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: "transparent",
              color: "var(--color-text-primary)",
              fontSize: "13px",
              cursor: "pointer",
            }}
          >
            {t("rules.createFirst")}
          </button>
        </div>
      )}

      {/* Rules list */}
      {rules.length > 0 && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "1px",
            borderRadius: "8px",
            overflow: "hidden",
            border: "1px solid var(--color-border)",
          }}
        >
          {rules.map((rule, index) => (
            <div key={rule.id}>
              {editingId === rule.id ? (
                <div style={{ padding: "1px 0" }}>{renderEditor()}</div>
              ) : (
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    padding: "14px 16px",
                    backgroundColor: "var(--color-bg)",
                    borderTop: index > 0 ? "1px solid var(--color-border)" : "none",
                  }}
                >
                  <div style={{ display: "flex", flexDirection: "column", gap: "2px" }}>
                    <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                      <span style={{ fontSize: "13px", fontWeight: 500, color: "var(--color-text-primary)" }}>
                        {rule.name}
                      </span>
                      <span
                        style={{
                          fontSize: "11px",
                          padding: "1px 6px",
                          borderRadius: "4px",
                          backgroundColor: rule.is_enabled ? "rgba(34,197,94,0.12)" : "rgba(156,163,175,0.12)",
                          color: rule.is_enabled ? "#22c55e" : "var(--color-text-secondary)",
                        }}
                      >
                        {rule.is_enabled ? t("rules.enabled") : t("rules.disabled")}
                      </span>
                    </div>
                    <span
                      style={{
                        fontSize: "12px",
                        color: "var(--color-text-secondary)",
                      }}
                    >
                      {t("rules.priorityLabel", { value: rule.priority })}
                    </span>
                  </div>
                  <div style={{ display: "flex", gap: "4px" }}>
                    <button
                      onClick={() => startEdit(rule)}
                      disabled={editingId !== null}
                      aria-label={t("rules.editRule")}
                      title={t("rules.editRule")}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        padding: "6px",
                        borderRadius: "6px",
                        border: "none",
                        backgroundColor: "transparent",
                        color: editingId !== null ? "var(--color-border)" : "var(--color-text-secondary)",
                        cursor: editingId !== null ? "not-allowed" : "pointer",
                      }}
                      onMouseEnter={(e) => {
                        if (editingId === null) {
                          e.currentTarget.style.color = "var(--color-accent)";
                          e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
                        }
                      }}
                      onMouseLeave={(e) => {
                        e.currentTarget.style.color = editingId !== null ? "var(--color-border)" : "var(--color-text-secondary)";
                        e.currentTarget.style.backgroundColor = "transparent";
                      }}
                    >
                      <Pencil size={15} />
                    </button>
                    <button
                      onClick={() => setDeleteTarget({ id: rule.id, name: rule.name })}
                      aria-label={t("rules.deleteRule")}
                      title={t("rules.deleteRule")}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        padding: "6px",
                        borderRadius: "6px",
                        border: "none",
                        backgroundColor: "transparent",
                        color: "var(--color-text-secondary)",
                        cursor: "pointer",
                      }}
                      onMouseEnter={(e) => {
                        e.currentTarget.style.color = "#ef4444";
                        e.currentTarget.style.backgroundColor = "rgba(239,68,68,0.08)";
                      }}
                      onMouseLeave={(e) => {
                        e.currentTarget.style.color = "var(--color-text-secondary)";
                        e.currentTarget.style.backgroundColor = "transparent";
                      }}
                    >
                      <Trash2 size={15} />
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Delete confirmation */}
      {deleteTarget && (
        <ConfirmDialog
          title={t("rules.deleteRule")}
          message={t("rules.confirmDelete", { name: deleteTarget.name })}
          destructive
          onCancel={() => setDeleteTarget(null)}
          onConfirm={() => {
            doDelete(deleteTarget.id);
            setDeleteTarget(null);
          }}
        />
      )}
    </div>
  );
}
