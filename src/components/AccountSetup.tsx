import { useState } from "react";
import { X } from "lucide-react";
import { addAccount, startSync } from "@/lib/api";
import type { AddAccountRequest } from "@/lib/api";
import { useMailStore } from "@/stores/mail.store";

const PRESETS: Record<
  string,
  Pick<
    AddAccountRequest,
    "imap_host" | "imap_port" | "smtp_host" | "smtp_port" | "use_tls"
  >
> = {
  gmail: {
    imap_host: "imap.gmail.com",
    imap_port: 993,
    smtp_host: "smtp.gmail.com",
    smtp_port: 587,
    use_tls: true,
  },
  outlook: {
    imap_host: "outlook.office365.com",
    imap_port: 993,
    smtp_host: "smtp.office365.com",
    smtp_port: 587,
    use_tls: true,
  },
  qq: {
    imap_host: "imap.qq.com",
    imap_port: 993,
    smtp_host: "smtp.qq.com",
    smtp_port: 465,
    use_tls: true,
  },
  "163": {
    imap_host: "imap.163.com",
    imap_port: 993,
    smtp_host: "smtp.163.com",
    smtp_port: 465,
    use_tls: true,
  },
};

interface Props {
  onClose: () => void;
}

export default function AccountSetup({ onClose }: Props) {
  const { fetchAccounts } = useMailStore();

  const [form, setForm] = useState<AddAccountRequest>({
    email: "",
    display_name: "",
    provider: "imap",
    imap_host: "",
    imap_port: 993,
    smtp_host: "",
    smtp_port: 587,
    username: "",
    password: "",
    use_tls: true,
  });

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function applyPreset(key: string) {
    const preset = PRESETS[key];
    if (!preset) return;
    setForm((prev) => ({ ...prev, ...preset }));
  }

  function handleChange(field: keyof AddAccountRequest, value: string | number | boolean) {
    setForm((prev) => {
      const updated = { ...prev, [field]: value };
      // Keep username in sync with email when username hasn't been manually changed
      if (field === "email" && prev.username === prev.email) {
        updated.username = value as string;
      }
      return updated;
    });
  }

  async function handleSubmit(e: React.SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      const account = await addAccount(form);
      await startSync(account.id);
      await fetchAccounts();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }

  const inputStyle: React.CSSProperties = {
    width: "100%",
    padding: "7px 10px",
    borderRadius: "6px",
    border: "1px solid var(--color-border)",
    backgroundColor: "var(--color-bg)",
    color: "var(--color-text-primary)",
    fontSize: "13px",
    outline: "none",
    boxSizing: "border-box",
  };

  const labelStyle: React.CSSProperties = {
    display: "block",
    fontSize: "12px",
    color: "var(--color-text-secondary)",
    marginBottom: "4px",
  };

  const fieldStyle: React.CSSProperties = {
    display: "flex",
    flexDirection: "column",
    gap: "0",
  };

  const rowStyle: React.CSSProperties = {
    display: "grid",
    gridTemplateColumns: "1fr 1fr",
    gap: "12px",
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        backgroundColor: "rgba(0,0,0,0.5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        style={{
          width: "480px",
          backgroundColor: "var(--color-bg)",
          borderRadius: "10px",
          boxShadow: "0 20px 60px rgba(0,0,0,0.3)",
          display: "flex",
          flexDirection: "column",
          maxHeight: "90vh",
          overflow: "hidden",
        }}
      >
        {/* Header */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "16px 20px",
            borderBottom: "1px solid var(--color-border)",
          }}
        >
          <h2
            style={{
              margin: 0,
              fontSize: "15px",
              fontWeight: 600,
              color: "var(--color-text-primary)",
            }}
          >
            Add Email Account
          </h2>
          <button
            onClick={onClose}
            style={{
              background: "none",
              border: "none",
              cursor: "pointer",
              padding: "4px",
              borderRadius: "4px",
              color: "var(--color-text-secondary)",
              display: "flex",
              alignItems: "center",
            }}
          >
            <X size={18} />
          </button>
        </div>

        {/* Scrollable body */}
        <div style={{ overflowY: "auto", padding: "20px" }}>
          {/* Preset buttons */}
          <div style={{ marginBottom: "20px" }}>
            <span style={{ ...labelStyle, marginBottom: "8px" }}>Quick setup</span>
            <div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}>
              {Object.keys(PRESETS).map((key) => (
                <button
                  key={key}
                  type="button"
                  onClick={() => applyPreset(key)}
                  style={{
                    padding: "5px 14px",
                    borderRadius: "20px",
                    border: "1px solid var(--color-border)",
                    backgroundColor: "transparent",
                    color: "var(--color-text-primary)",
                    fontSize: "12px",
                    cursor: "pointer",
                    textTransform: "capitalize",
                  }}
                >
                  {key === "163" ? "163" : key.charAt(0).toUpperCase() + key.slice(1)}
                </button>
              ))}
            </div>
          </div>

          <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
            {/* Email */}
            <div style={fieldStyle}>
              <label style={labelStyle}>Email address</label>
              <input
                style={inputStyle}
                type="email"
                required
                value={form.email}
                onChange={(e) => handleChange("email", e.target.value)}
                placeholder="you@example.com"
              />
            </div>

            {/* Display name */}
            <div style={fieldStyle}>
              <label style={labelStyle}>Display name</label>
              <input
                style={inputStyle}
                type="text"
                required
                value={form.display_name}
                onChange={(e) => handleChange("display_name", e.target.value)}
                placeholder="Your Name"
              />
            </div>

            {/* IMAP */}
            <div style={rowStyle}>
              <div style={fieldStyle}>
                <label style={labelStyle}>IMAP host</label>
                <input
                  style={inputStyle}
                  type="text"
                  required
                  value={form.imap_host}
                  onChange={(e) => handleChange("imap_host", e.target.value)}
                  placeholder="imap.example.com"
                />
              </div>
              <div style={fieldStyle}>
                <label style={labelStyle}>IMAP port</label>
                <input
                  style={inputStyle}
                  type="number"
                  required
                  value={form.imap_port}
                  onChange={(e) => handleChange("imap_port", parseInt(e.target.value, 10))}
                />
              </div>
            </div>

            {/* SMTP */}
            <div style={rowStyle}>
              <div style={fieldStyle}>
                <label style={labelStyle}>SMTP host</label>
                <input
                  style={inputStyle}
                  type="text"
                  required
                  value={form.smtp_host}
                  onChange={(e) => handleChange("smtp_host", e.target.value)}
                  placeholder="smtp.example.com"
                />
              </div>
              <div style={fieldStyle}>
                <label style={labelStyle}>SMTP port</label>
                <input
                  style={inputStyle}
                  type="number"
                  required
                  value={form.smtp_port}
                  onChange={(e) => handleChange("smtp_port", parseInt(e.target.value, 10))}
                />
              </div>
            </div>

            {/* Username */}
            <div style={fieldStyle}>
              <label style={labelStyle}>Username</label>
              <input
                style={inputStyle}
                type="text"
                required
                value={form.username}
                onChange={(e) => handleChange("username", e.target.value)}
                placeholder="Defaults to email address"
              />
            </div>

            {/* Password */}
            <div style={fieldStyle}>
              <label style={labelStyle}>Password / App password</label>
              <input
                style={inputStyle}
                type="password"
                required
                value={form.password}
                onChange={(e) => handleChange("password", e.target.value)}
              />
            </div>

            {/* Use TLS */}
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                cursor: "pointer",
              }}
              onClick={() => handleChange("use_tls", !form.use_tls)}
            >
              <input
                type="checkbox"
                id="use_tls"
                checked={form.use_tls}
                onChange={(e) => handleChange("use_tls", e.target.checked)}
                style={{ cursor: "pointer" }}
              />
              <label
                htmlFor="use_tls"
                style={{
                  fontSize: "13px",
                  color: "var(--color-text-primary)",
                  cursor: "pointer",
                }}
              >
                Use TLS
              </label>
            </div>

            {/* Error */}
            {error && (
              <div
                style={{
                  padding: "10px 12px",
                  borderRadius: "6px",
                  backgroundColor: "rgba(239,68,68,0.1)",
                  border: "1px solid rgba(239,68,68,0.3)",
                  color: "#ef4444",
                  fontSize: "13px",
                }}
              >
                {error}
              </div>
            )}

            {/* Submit */}
            <button
              type="submit"
              disabled={loading}
              style={{
                padding: "9px 16px",
                borderRadius: "6px",
                border: "none",
                backgroundColor: "var(--color-accent)",
                color: "#fff",
                fontSize: "13px",
                fontWeight: 600,
                cursor: loading ? "not-allowed" : "pointer",
                opacity: loading ? 0.7 : 1,
                marginTop: "4px",
              }}
            >
              {loading ? "Adding account…" : "Add Account & Sync"}
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}
