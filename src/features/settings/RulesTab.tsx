import { Fragment, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Pencil, Trash2, ShieldCheck, X } from "lucide-react";
import { createRule, deleteRule, listRules, updateRule, type Rule } from "@/lib/api";
import ConfirmDialog from "@/components/ConfirmDialog";
import IconButton from "@/components/ui/IconButton";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";
import {
  SettingsList,
  SettingsListRow,
  SettingsNotice,
  SettingsSection,
  StatusChip,
} from "./SettingsLayout";
import { useToastStore } from "@/stores/toast.store";
import { useUIStore } from "@/stores/ui.store";
import { extractErrorMessage, formatError } from "@/lib/extractErrorMessage";
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

function buildRuleFormFromSelection(text: string, name: string): RuleFormData {
  const value = text.trim();
  return {
    name,
    priority: 0,
    is_enabled: true,
    conditions: [{ field: "body", op: "contains", value }],
    actions: [{ type: "Archive" }],
  };
}

export default function RulesTab() {
  const { t } = useTranslation();
  const addToast = useToastStore((s) => s.addToast);
  const pendingRuleDraftText = useUIStore((s) => s.pendingRuleDraftText);
  const setPendingRuleDraftText = useUIStore((s) => s.setPendingRuleDraftText);
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

  useEffect(() => {
    if (!pendingRuleDraftText) return;
    setEditingId("__new__");
    setForm(buildRuleFormFromSelection(
      pendingRuleDraftText,
      t("rules.contextRuleName", "Selected text rule"),
    ));
    setError(null);
    setPendingRuleDraftText(null);
  }, [pendingRuleDraftText, setPendingRuleDraftText, t]);

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
    if (form.conditions.some((c) => !c.value.trim())) {
      setError(t("rules.conditionValueRequired", "Condition values cannot be empty."));
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
      setError(formatError(err, { fallbackKey: "errors.saveRule" }));
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

  // ── Condition row renderer ──────────────────────────────────
  function renderConditionRow(condition: Condition, index: number) {
    return (
      <div key={index} style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
        <select
          aria-label={t("rules.conditionField", "Condition field")}
          className="select"
          value={condition.field}
          onChange={(e) => updateCondition(index, { field: e.target.value as ConditionField })}
          style={{ flex: "0 0 120px" }}
        >
          {CONDITION_FIELDS.map((f) => (
            <option key={f} value={f}>{t(`rules.field_${f}`, f)}</option>
          ))}
        </select>
        <select
          aria-label={t("rules.conditionOp", "Condition operator")}
          className="select"
          value={condition.op}
          onChange={(e) => updateCondition(index, { op: e.target.value as ConditionOp })}
          style={{ flex: "0 0 132px" }}
        >
          {CONDITION_OPS.map((op) => (
            <option key={op} value={op}>{t(`rules.op_${op}`, op)}</option>
          ))}
        </select>
        <input
          type="text"
          className="input"
          aria-label={t("rules.conditionValue", "Condition value")}
          value={condition.value}
          onChange={(e) => updateCondition(index, { value: e.target.value })}
          placeholder={t("rules.valuePlaceholder", "Value")}
          style={{ flex: 1 }}
        />
        <IconButton
          size="md"
          label={t("common.remove", "Remove")}
          icon={<X size={14} />}
          onClick={() => removeCondition(index)}
          disabled={form.conditions.length <= 1}
        />
      </div>
    );
  }

  // ── Action row renderer ─────────────────────────────────────
  function renderActionRow(action: RuleAction, index: number) {
    const needsValue = action.type === "AddLabel" || action.type === "MoveToFolder" || action.type === "SetKanbanColumn";

    return (
      <div key={index} style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
        <select
          aria-label={t("rules.actionType", "Action type")}
          className="select"
          value={action.type}
          onChange={(e) => {
            const newType = e.target.value as ActionType;
            const val = newType === "SetKanbanColumn" ? "todo" : "";
            updateAction(index, { type: newType, value: val });
          }}
          style={{ flex: "0 0 160px" }}
        >
          {ACTION_TYPES.map((at) => (
            <option key={at} value={at}>{t(`rules.action_${at}`, at)}</option>
          ))}
        </select>
        {needsValue && (
          action.type === "SetKanbanColumn" ? (
            <select
              aria-label={t("rules.kanbanColumn", "Kanban column")}
              className="select"
              value={action.value || "todo"}
              onChange={(e) => updateAction(index, { value: e.target.value })}
              style={{ flex: 1 }}
            >
              {KANBAN_COLUMNS.map((c) => (
                <option key={c} value={c}>{t(`rules.kanban_${c}`, c)}</option>
              ))}
            </select>
          ) : (
            <input
              type="text"
              className="input"
              aria-label={t("rules.actionValue", "Action value")}
              value={action.value || ""}
              onChange={(e) => updateAction(index, { value: e.target.value })}
              placeholder={
                action.type === "AddLabel"
                  ? t("rules.labelPlaceholder", "Label name")
                  : t("rules.folderPlaceholder", "Folder name")
              }
              style={{ flex: 1 }}
            />
          )
        )}
        {!needsValue && <div style={{ flex: 1 }} />}
        <IconButton
          size="md"
          label={t("common.remove", "Remove")}
          icon={<X size={14} />}
          onClick={() => removeAction(index)}
          disabled={form.actions.length <= 1}
        />
      </div>
    );
  }

  // ── Editor panel (rendered as a stacked row of the rules list) ──
  function renderEditor() {
    return (
      <div className="settings-list__row settings-list__row--stacked" style={{ gap: "var(--space-4)" }}>
        <div className="settings-row">
          <div className="field">
            <label htmlFor="rule-name" className="field__label">{t("rules.name")}</label>
            <input
              id="rule-name"
              type="text"
              className="input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder={t("rules.namePlaceholder")}
            />
          </div>
          <div className="field field--narrow">
            <label htmlFor="rule-priority" className="field__label">{t("rules.priority")}</label>
            <input
              id="rule-priority"
              type="number"
              className="input"
              value={form.priority}
              onChange={(e) => setForm({ ...form, priority: Number(e.target.value) })}
            />
          </div>
        </div>

        <Checkbox
          checked={form.is_enabled}
          onChange={(e) => setForm({ ...form, is_enabled: e.target.checked })}
          label={t("rules.enabled")}
        />

        <div className="field">
          <span className="field__label">{t("rules.conditions", "Conditions")}</span>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
            {form.conditions.map((c, i) => renderConditionRow(c, i))}
          </div>
          <div>
            <Button size="sm" variant="ghost" icon={<Plus size={14} />} onClick={addCondition}>
              {t("rules.addCondition", "Add condition")}
            </Button>
          </div>
        </div>

        <div className="field">
          <span className="field__label">{t("rules.actions", "Actions")}</span>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
            {form.actions.map((a, i) => renderActionRow(a, i))}
          </div>
          <div>
            <Button size="sm" variant="ghost" icon={<Plus size={14} />} onClick={addAction}>
              {t("rules.addAction", "Add action")}
            </Button>
          </div>
        </div>

        {error && <p role="alert" className="settings-error">{error}</p>}

        <div className="settings-actions settings-actions--end">
          <Button onClick={cancelEdit}>{t("common.cancel")}</Button>
          <Button variant="primary" onClick={handleSave}>{t("common.save")}</Button>
        </div>
      </div>
    );
  }

  return (
    <div>
      <SettingsSection
        title={t("rules.listTitle", "Your rules")}
        actions={
          <Button size="sm" icon={<Plus size={14} />} onClick={startCreate} disabled={editingId !== null}>
            {t("rules.addRule")}
          </Button>
        }
      >
        {editingId === "__new__" && <SettingsList>{renderEditor()}</SettingsList>}

        {fetchError && editingId !== "__new__" && (
          <SettingsNotice tone="danger" className="fade-in" role="alert">
            <strong style={{ color: "inherit" }}>{t("rules.fetchError", "Failed to load rules")}</strong>
            <div style={{ marginTop: "var(--space-1)" }}>{fetchError}</div>
            <div style={{ marginTop: "var(--space-2)" }}>
              <Button size="sm" onClick={fetchRules}>{t("common.retry", "Retry")}</Button>
            </div>
          </SettingsNotice>
        )}

        {rules.length === 0 && !fetchError && editingId !== "__new__" && (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: "var(--space-3)",
              padding: "var(--space-8) 0",
              color: "var(--color-text-tertiary)",
            }}
          >
            <ShieldCheck size={40} strokeWidth={1.5} aria-hidden="true" />
            <p className="settings-empty">{t("rules.noRules")}</p>
            <Button onClick={startCreate}>{t("rules.createFirst")}</Button>
          </div>
        )}

        {rules.length > 0 && (
          <SettingsList>
            {rules.map((rule) => (
              editingId === rule.id ? (
                <Fragment key={rule.id}>{renderEditor()}</Fragment>
              ) : (
                <SettingsListRow
                  key={rule.id}
                  title={
                    <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--space-2)", maxWidth: "100%" }}>
                      <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{rule.name}</span>
                      <StatusChip tone={rule.is_enabled ? "success" : "neutral"}>
                        {rule.is_enabled ? t("rules.enabled") : t("rules.disabled")}
                      </StatusChip>
                    </span>
                  }
                  meta={t("rules.priorityLabel", { value: rule.priority })}
                  actions={
                    <>
                      <IconButton
                        label={t("rules.editRule")}
                        icon={<Pencil size={14} />}
                        disabled={editingId !== null}
                        onClick={() => startEdit(rule)}
                      />
                      <IconButton
                        variant="danger"
                        label={t("rules.deleteRule")}
                        icon={<Trash2 size={14} />}
                        onClick={() => setDeleteTarget({ id: rule.id, name: rule.name })}
                      />
                    </>
                  }
                />
              )
            ))}
          </SettingsList>
        )}
      </SettingsSection>

      {deleteTarget && (
        <ConfirmDialog
          title={t("rules.deleteRule")}
          message={t("rules.confirmDelete", { name: deleteTarget.name })}
          confirmLabel={t("rules.deleteRule")}
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
