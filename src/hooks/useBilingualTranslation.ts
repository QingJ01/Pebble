import { useState } from "react";
import { translateText } from "@/lib/api";
import { sanitizeHtml } from "@/lib/sanitizeHtml";
import type { Message, RenderedHtml, TranslateResult } from "@/lib/api";
import { toastError } from "@/stores/toast.store";
import { profileLocalStorage } from "@/lib/profileStorage";

// Translation cache: avoids re-translating on toggle or revisit (capped at 20 entries)
const translationCache = new Map<string, TranslateResult & { _isHtml?: boolean }>();
const TRANSLATION_CACHE_MAX = 20;
const CHUNK_SIZE = 30; // Max text nodes per translation request

export function useBilingualTranslation(
  messageId: string | null,
  rendered: RenderedHtml | null,
  message: Message | null,
) {
  const [bilingualMode, setBilingualMode] = useState(false);
  const [bilingualResult, setBilingualResult] = useState<TranslateResult | null>(null);
  const [bilingualLoading, setBilingualLoading] = useState(false);

  async function handleBilingualToggle() {
    if (bilingualMode) {
      setBilingualMode(false);
      return;
    }
    if (!message || !messageId) return;

    const uiLang = profileLocalStorage.getItem("pebble-language") || "zh";
    const cacheKey = `${messageId}:${uiLang}`;

    // Check cache first
    const cached = translationCache.get(cacheKey);
    if (cached) {
      setBilingualResult(cached);
      setBilingualMode(true);
      return;
    }

    setBilingualMode(true);
    setBilingualLoading(true);
    try {
      const hasHtml = !!(rendered && rendered.html);

      if (hasHtml) {
        // HTML email: translate in chunks while preserving layout
        const doc = new DOMParser().parseFromString(sanitizeHtml(rendered!.html), "text/html");
        const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
        const textNodes: Text[] = [];
        let n: Text | null;
        while ((n = walker.nextNode() as Text | null)) {
          if (n.textContent?.trim()) textNodes.push(n);
        }

        // Translate in chunks to avoid timeouts on long emails.
        // Uses a unique separator so we can reliably split the response,
        // with numbered-index fallback for services that preserve them.
        const SEP = "\n⸻\n";
        for (let start = 0; start < textNodes.length; start += CHUNK_SIZE) {
          const chunk = textNodes.slice(start, start + CHUNK_SIZE);
          const batch = chunk.map((nd) => nd.textContent!.trim()).join(SEP);
          const result = await translateText(batch, "auto", uiLang);

          // Split on separator; if the service preserved it, we get exact mapping
          const parts = result.translated.split("⸻").map((s) => s.trim()).filter(Boolean);
          if (parts.length === chunk.length) {
            // Exact 1:1 mapping
            for (let i = 0; i < chunk.length; i++) {
              chunk[i].textContent = parts[i];
            }
          } else {
            // Fallback: replace the entire chunk's text with the translated result
            // Split by newlines and try positional matching
            const lines = result.translated.split("\n").map((s) => s.trim()).filter(Boolean);
            for (let i = 0; i < Math.min(chunk.length, lines.length); i++) {
              chunk[i].textContent = lines[i];
            }
          }
          // Show progressive results after each chunk
          const partial = { translated: sanitizeHtml(doc.body.innerHTML), segments: [], _isHtml: true } as TranslateResult & { _isHtml?: boolean };
          setBilingualResult(partial);
        }

        const final_ = { translated: sanitizeHtml(doc.body.innerHTML), segments: [], _isHtml: true } as TranslateResult & { _isHtml?: boolean };
        setBilingualResult(final_);
        if (translationCache.size >= TRANSLATION_CACHE_MAX) {
          translationCache.delete(translationCache.keys().next().value!);
        }
        translationCache.set(cacheKey, final_);
      } else {
        // Plain text email
        const textToTranslate = message.body_text
          || new DOMParser().parseFromString(message.body_html_raw || "", "text/html").body.textContent
          || "";
        const result = { ...await translateText(textToTranslate, "auto", uiLang), _isHtml: false } as TranslateResult & { _isHtml?: boolean };
        setBilingualResult(result);
        if (translationCache.size >= TRANSLATION_CACHE_MAX) {
          translationCache.delete(translationCache.keys().next().value!);
        }
        translationCache.set(cacheKey, result);
      }
    } catch (err) {
      console.error("Translation failed:", err);
      toastError(err, "errors.translate");
    } finally {
      setBilingualLoading(false);
    }
  }

  /** Reset bilingual state (call when messageId changes) */
  function resetBilingual() {
    setBilingualMode(false);
    setBilingualResult(null);
  }

  return { bilingualMode, bilingualResult, bilingualLoading, handleBilingualToggle, resetBilingual };
}
