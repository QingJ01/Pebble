import { useState, useRef, useEffect, useCallback, useId } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import {
  searchContactSuggestions,
  suppressContactSuggestion,
  type ContactSuggestion,
} from "@/lib/api";
import { queryClient } from "@/lib/query-client";
import { contactSuggestionsQueryRoot } from "@/hooks/queries";
import { useToastStore } from "@/stores/toast.store";
import { isValidEmailAddress } from "@/features/compose/recipient-utils";
import OverlayPortal from "./ui/OverlayPortal";
import { useFloatingPosition } from "@/hooks/useFloatingPosition";
import Button from "./ui/Button";
import "./ui/overlay.css";

interface ContactAutocompleteProps {
  value: string[];
  onChange: (addresses: string[]) => void;
  accountId: string;
  id?: string;
  name?: string;
  ariaLabelledBy?: string;
  autoComplete?: string;
  placeholder?: string;
  inputValue?: string;
  onInputValueChange?: (value: string) => void;
}

export default function ContactAutocomplete({
  value,
  onChange,
  accountId,
  id,
  name,
  ariaLabelledBy,
  autoComplete = "email",
  placeholder,
  inputValue: controlledInputValue,
  onInputValueChange,
}: ContactAutocompleteProps) {
  const { t } = useTranslation();
  const instanceId = useId();
  const [uncontrolledInputValue, setUncontrolledInputValue] = useState("");
  const inputValue = controlledInputValue ?? uncontrolledInputValue;
  const [suggestions, setSuggestions] = useState<ContactSuggestion[]>([]);
  const [showDropdown, setShowDropdown] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  useFloatingPosition({
    open: showDropdown,
    floatingRef: dropdownRef,
    anchorRef: containerRef,
    placement: "bottom-start",
    gap: 2,
    matchWidth: true,
  });
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchGenerationRef = useRef(0);
  const activeAccountIdRef = useRef(accountId);
  const inputValueRef = useRef(inputValue);
  const selectedAddressesRef = useRef(value);
  activeAccountIdRef.current = accountId;
  inputValueRef.current = inputValue;
  selectedAddressesRef.current = value;

  const setInputValue = useCallback(
    (nextValue: string) => {
      if (controlledInputValue === undefined) {
        setUncontrolledInputValue(nextValue);
      }
      onInputValueChange?.(nextValue);
    },
    [controlledInputValue, onInputValueChange],
  );

  const fetchSuggestions = useCallback(
    async (query: string) => {
      const generation = ++searchGenerationRef.current;
      const requestIsCurrent = () => searchGenerationRef.current === generation
        && activeAccountIdRef.current === accountId
        && inputValueRef.current === query;
      if (!query.trim() || !accountId) {
        setSuggestions([]);
        setShowDropdown(false);
        setLoading(false);
        return;
      }
      setLoading(true);
      try {
        const results = await searchContactSuggestions(accountId, query, 10);
        if (!requestIsCurrent()) return;
        const selectedAddresses = new Set(
          selectedAddressesRef.current.map((address) => address.trim().toLowerCase()),
        );
        const filtered = results.filter(
          (contact) => !selectedAddresses.has(contact.address.trim().toLowerCase()),
        );
        setSuggestions(filtered);
        setShowDropdown(filtered.length > 0);
        setActiveIndex(-1);
      } catch {
        if (!requestIsCurrent()) return;
        setSuggestions([]);
        setShowDropdown(false);
      } finally {
        if (requestIsCurrent()) {
          setLoading(false);
        }
      }
    },
    [accountId],
  );

  const handleInputChange = (text: string) => {
    setInputValue(text);
    searchGenerationRef.current += 1;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      fetchSuggestions(text);
    }, 200);
  };

  const selectContact = (contact: ContactSuggestion) => {
    const normalized = contact.address.trim().toLowerCase();
    if (!value.some((address) => address.trim().toLowerCase() === normalized)) {
      onChange([...value, contact.address]);
    }
    setInputValue("");
    searchGenerationRef.current += 1;
    setSuggestions([]);
    setShowDropdown(false);
    setActiveIndex(-1);
    inputRef.current?.focus();
  };

  const addRawAddress = (text: string) => {
    const trimmed = text.trim();
    if (!trimmed) { setInputValue(""); return; }
    if (isValidEmailAddress(trimmed) && !value.some(
      (address) => address.trim().toLowerCase() === trimmed.toLowerCase(),
    )) {
      onChange([...value, trimmed]);
    } else if (!isValidEmailAddress(trimmed)) {
      useToastStore.getState().addToast({
        message: t("compose.invalidEmail", "Invalid email address"),
        type: "error",
      });
    }
    setInputValue("");
    searchGenerationRef.current += 1;
    setSuggestions([]);
    setShowDropdown(false);
  };

  const removeChip = (address: string) => {
    onChange(value.filter((a) => a !== address));
    inputRef.current?.focus();
  };

  const removeSuggestion = async (
    event: React.MouseEvent<HTMLButtonElement>,
    contact: ContactSuggestion,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    try {
      await suppressContactSuggestion(contact.address);
      setSuggestions((current) => {
        const next = current.filter((item) => (
          item.address.trim().toLowerCase() !== contact.address.trim().toLowerCase()
        ));
        if (next.length === 0) setShowDropdown(false);
        return next;
      });
      setActiveIndex(-1);
      await queryClient.invalidateQueries({ queryKey: contactSuggestionsQueryRoot });
      inputRef.current?.focus();
    } catch {
      useToastStore.getState().addToast({
        message: t("compose.removeSuggestionFailed", "Failed to remove suggestion"),
        type: "error",
      });
    }
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
      if (showDropdown) {
        // Close only the suggestions; don't let Escape also close the composer.
        e.stopPropagation();
      }
      setShowDropdown(false);
      setActiveIndex(-1);
    } else if (e.key === "Backspace" && !inputValue && value.length > 0) {
      removeChip(value[value.length - 1]);
    } else if (e.key === "," || e.key === "Tab") {
      if (inputValue.trim()) {
        const activeSuggestion = suggestions[activeIndex];
        const removableSuggestion = activeIndex >= 0
          ? (activeSuggestion?.source === "recent" ? activeSuggestion : undefined)
          : suggestions.find((suggestion) => suggestion.source === "recent");
        if (e.key === "Tab" && showDropdown && removableSuggestion) {
          // Keep the removal button mounted while native Tab moves focus to it.
          return;
        }
        e.preventDefault();
        addRawAddress(inputValue);
      }
    }
  };

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        containerRef.current &&
        !containerRef.current.contains(target) &&
        !dropdownRef.current?.contains(target)
      ) {
        setShowDropdown(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  useEffect(() => {
    searchGenerationRef.current += 1;
    if (debounceRef.current) {
      clearTimeout(debounceRef.current);
      debounceRef.current = null;
    }
    setSuggestions([]);
    setShowDropdown(false);
    setActiveIndex(-1);
    setLoading(false);
  }, [accountId]);

  // Cleanup debounce on unmount
  useEffect(() => {
    return () => {
      searchGenerationRef.current += 1;
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

  const removableSuggestion = activeIndex >= 0
    ? (suggestions[activeIndex]?.source === "recent" ? suggestions[activeIndex] : undefined)
    : suggestions.find((suggestion) => suggestion.source === "recent");

  return (
    <div ref={containerRef} className="contact-autocomplete"
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          setShowDropdown(false);
          setActiveIndex(-1);
        }
      }}>
        <div
          className="scroll-region contact-autocomplete-scroll contact-autocomplete__field"
        onClick={() => inputRef.current?.focus()}
      >
        {value.map((addr) => (
          <span key={addr} className="chip chip--md contact-chip">
            <span className="contact-chip__label">{addr}</span>
            <button
              type="button"
              className="chip__remove"
              onClick={(e) => {
                e.stopPropagation();
                removeChip(addr);
              }}
              aria-label={t("common.remove")}
            >
              <X size={11} aria-hidden="true" />
            </button>
          </span>
        ))}
        <input
          id={id}
          name={name}
          ref={inputRef}
          type="text"
          autoComplete={autoComplete}
          value={inputValue}
          onChange={(e) => handleInputChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onFocus={() => {
            if (inputValue.trim() && suggestions.length > 0) {
              setShowDropdown(true);
            }
          }}
          placeholder={value.length === 0 ? placeholder : undefined}
          role="combobox"
          aria-labelledby={ariaLabelledBy}
          aria-autocomplete="list"
          aria-expanded={showDropdown && suggestions.length > 0}
          aria-controls={`${instanceId}-listbox`}
          aria-activedescendant={activeIndex >= 0 ? `${instanceId}-option-${activeIndex}` : undefined}
          className="contact-autocomplete__input"
        />
      </div>

      {showDropdown && (
        <OverlayPortal>
        <div
          ref={dropdownRef}
          className="popover-panel anim-popover-in contact-suggestions"
          onMouseDown={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
        >
          <div
            id={`${instanceId}-listbox`}
            role="listbox"
            className="contact-suggestions__list"
          >
          {loading ? (
            <div className="contact-suggestions__note">
              {t("common.loading")}
            </div>
          ) : suggestions.length === 0 ? (
            <div className="contact-suggestions__note">
              {t("compose.noContactsFound")}
            </div>
          ) : (
            suggestions.map((contact, idx) => (
              <div
                key={contact.address}
                role="option"
                id={`${instanceId}-option-${idx}`}
                aria-label={`${contact.name ?? contact.address} ${contact.address}`}
                aria-selected={idx === activeIndex}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => selectContact(contact)}
                onMouseEnter={() => setActiveIndex(idx)}
                className={`menu-item contact-suggestion${idx === activeIndex ? " is-active" : ""}`}
              >
                <div className="contact-suggestion__text">
                  {contact.name && (
                    <div className="contact-suggestion__name">
                      {highlightMatch(contact.name, inputValue)}
                    </div>
                  )}
                  <div className="contact-suggestion__address">
                    {highlightMatch(contact.address, inputValue)}
                  </div>
                </div>
                <span className="contact-suggestion__source">
                  {contact.source === "saved"
                    ? t("compose.savedContact", "Saved contact")
                    : t("compose.recentContact", "Recent")}
                </span>
              </div>
            ))
          )}
          </div>
          {removableSuggestion && (
            <Button
              variant="ghost"
              size="sm"
              className="contact-suggestions__remove"
              aria-label={t("compose.removeSuggestionFor", { address: removableSuggestion.address, defaultValue: "Remove suggestion: {{address}}" })}
              icon={<X size={12} />}
              onClick={(event) => removeSuggestion(event, removableSuggestion)}
            >
              {t("compose.removeSuggestionFor", { address: removableSuggestion.address, defaultValue: "Remove suggestion: {{address}}" })}
            </Button>
          )}
        </div>
        </OverlayPortal>
      )}
    </div>
  );
}
