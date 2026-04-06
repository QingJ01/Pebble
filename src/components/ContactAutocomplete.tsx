import { useState, useRef, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { searchContacts, type KnownContact } from "@/lib/api";

interface ContactAutocompleteProps {
  value: string[];
  onChange: (addresses: string[]) => void;
  accountId: string;
  placeholder?: string;
}

export default function ContactAutocomplete({
  value,
  onChange,
  accountId,
  placeholder,
}: ContactAutocompleteProps) {
  const { t } = useTranslation();
  const [inputValue, setInputValue] = useState("");
  const [suggestions, setSuggestions] = useState<KnownContact[]>([]);
  const [showDropdown, setShowDropdown] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchSuggestions = useCallback(
    async (query: string) => {
      if (!query.trim() || !accountId) {
        setSuggestions([]);
        setShowDropdown(false);
        return;
      }
      setLoading(true);
      try {
        const results = await searchContacts(accountId, query, 10);
        // Filter out already-selected addresses
        const filtered = results.filter(
          (c) => !value.includes(c.address),
        );
        setSuggestions(filtered);
        setShowDropdown(filtered.length > 0);
        setActiveIndex(-1);
      } catch {
        setSuggestions([]);
        setShowDropdown(false);
      } finally {
        setLoading(false);
      }
    },
    [accountId, value],
  );

  const handleInputChange = (text: string) => {
    setInputValue(text);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      fetchSuggestions(text);
    }, 200);
  };

  const selectContact = (contact: KnownContact) => {
    if (!value.includes(contact.address)) {
      onChange([...value, contact.address]);
    }
    setInputValue("");
    setSuggestions([]);
    setShowDropdown(false);
    setActiveIndex(-1);
    inputRef.current?.focus();
  };

  const addRawAddress = (text: string) => {
    const trimmed = text.trim();
    // Basic email format validation
    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (trimmed && emailRegex.test(trimmed) && !value.includes(trimmed)) {
      onChange([...value, trimmed]);
    }
    setInputValue("");
    setSuggestions([]);
    setShowDropdown(false);
  };

  const removeChip = (address: string) => {
    onChange(value.filter((a) => a !== address));
    inputRef.current?.focus();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (showDropdown && suggestions.length > 0) {
        setActiveIndex((prev) =>
          prev < suggestions.length - 1 ? prev + 1 : 0,
        );
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (showDropdown && suggestions.length > 0) {
        setActiveIndex((prev) =>
          prev > 0 ? prev - 1 : suggestions.length - 1,
        );
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (activeIndex >= 0 && activeIndex < suggestions.length) {
        selectContact(suggestions[activeIndex]);
      } else if (inputValue.trim()) {
        addRawAddress(inputValue);
      }
    } else if (e.key === "Escape") {
      setShowDropdown(false);
      setActiveIndex(-1);
    } else if (e.key === "Backspace" && !inputValue && value.length > 0) {
      removeChip(value[value.length - 1]);
    } else if (e.key === "," || e.key === "Tab") {
      if (inputValue.trim()) {
        e.preventDefault();
        addRawAddress(inputValue);
      }
    }
  };

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setShowDropdown(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // Cleanup debounce on unmount
  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  const highlightMatch = (text: string, query: string) => {
    if (!query.trim()) return text;
    const idx = text.toLowerCase().indexOf(query.toLowerCase());
    if (idx === -1) return text;
    return (
      <>
        {text.slice(0, idx)}
        <strong>{text.slice(idx, idx + query.length)}</strong>
        {text.slice(idx + query.length)}
      </>
    );
  };

  return (
    <div ref={containerRef} style={{ position: "relative", flex: 1 }}>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "4px",
          alignItems: "center",
          padding: "4px 8px",
          minHeight: "32px",
        }}
        onClick={() => inputRef.current?.focus()}
      >
        {value.map((addr) => (
          <span
            key={addr}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: "4px",
              padding: "2px 8px",
              backgroundColor: "var(--color-bg-secondary, #f0f0f0)",
              borderRadius: "12px",
              fontSize: "12px",
              color: "var(--color-text-primary)",
              border: "1px solid var(--color-border)",
            }}
          >
            {addr}
            <button
              onClick={(e) => {
                e.stopPropagation();
                removeChip(addr);
              }}
              style={{
                background: "none",
                border: "none",
                cursor: "pointer",
                padding: "0 2px",
                fontSize: "12px",
                color: "var(--color-text-secondary)",
                lineHeight: 1,
              }}
              aria-label={t("common.remove")}
            >
              ×
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          type="text"
          value={inputValue}
          onChange={(e) => handleInputChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onFocus={() => {
            if (inputValue.trim() && suggestions.length > 0) {
              setShowDropdown(true);
            }
          }}
          placeholder={value.length === 0 ? placeholder : undefined}
          style={{
            flex: 1,
            minWidth: "120px",
            border: "none",
            outline: "none",
            backgroundColor: "transparent",
            fontSize: "13px",
            color: "var(--color-text-primary)",
            padding: "4px 0",
          }}
        />
      </div>

      {showDropdown && (
        <div
          style={{
            position: "absolute",
            top: "100%",
            left: 0,
            right: 0,
            zIndex: 1100,
            backgroundColor: "var(--color-bg)",
            border: "1px solid var(--color-border)",
            borderRadius: "8px",
            boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
            maxHeight: "200px",
            overflowY: "auto",
            marginTop: "2px",
          }}
        >
          {loading ? (
            <div
              style={{
                padding: "8px 12px",
                fontSize: "12px",
                color: "var(--color-text-secondary)",
              }}
            >
              {t("common.loading")}
            </div>
          ) : suggestions.length === 0 ? (
            <div
              style={{
                padding: "8px 12px",
                fontSize: "12px",
                color: "var(--color-text-secondary)",
              }}
            >
              {t("compose.noContactsFound")}
            </div>
          ) : (
            suggestions.map((contact, idx) => (
              <div
                key={contact.address}
                onClick={() => selectContact(contact)}
                onMouseEnter={() => setActiveIndex(idx)}
                style={{
                  padding: "6px 12px",
                  cursor: "pointer",
                  backgroundColor:
                    idx === activeIndex
                      ? "var(--color-bg-secondary, #f5f5f5)"
                      : "transparent",
                  fontSize: "13px",
                }}
              >
                {contact.name && (
                  <div
                    style={{
                      color: "var(--color-text-primary)",
                      fontWeight: 500,
                    }}
                  >
                    {highlightMatch(contact.name, inputValue)}
                  </div>
                )}
                <div
                  style={{
                    color: "var(--color-text-secondary)",
                    fontSize: "12px",
                  }}
                >
                  {highlightMatch(contact.address, inputValue)}
                </div>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
