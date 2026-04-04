import { useState } from "react";
import { searchMessages } from "@/lib/api";
import type { SearchHit } from "@/lib/api";

export function useSearch() {
  const [results, setResults] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");

  async function search(q: string) {
    setQuery(q);
    setLoading(true);
    try {
      const hits = await searchMessages(q);
      setResults(hits);
    } finally {
      setLoading(false);
    }
  }

  function clear() {
    setResults([]);
    setQuery("");
  }

  return { results, loading, query, search, clear };
}
