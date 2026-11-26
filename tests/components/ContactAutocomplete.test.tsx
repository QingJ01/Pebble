import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ContactAutocomplete from "../../src/components/ContactAutocomplete";
import {
  searchContactSuggestions,
  suppressContactSuggestion,
  type ContactSuggestion,
} from "../../src/lib/api";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, options?: unknown) => {
      if (typeof options === "string") return options;
      const opts = (options ?? {}) as Record<string, unknown>;
      const template = typeof opts.defaultValue === "string" ? opts.defaultValue : key;
      return template.replace(/\{\{(\w+)\}\}/g, (_m: string, n: string) => String(opts[n] ?? ""));
    } }),
}));

vi.mock("../../src/lib/api", () => ({
  searchContactSuggestions: vi.fn(),
  suppressContactSuggestion: vi.fn(),
}));

vi.mock("../../src/stores/toast.store", () => ({
  useToastStore: { getState: () => ({ addToast: vi.fn() }) },
}));

const searchContactSuggestionsMock = vi.mocked(searchContactSuggestions);

function suggestion(overrides: Partial<ContactSuggestion>): ContactSuggestion {
  return {
    contact_id: null,
    name: null,
    address: "person@example.com",
    source: "recent",
    is_favorite: false,
    last_interaction_at: 1,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("ContactAutocomplete", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    searchContactSuggestionsMock.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("forwards form identity and label association to the combobox input", () => {
    render(
      <>
        <label id="to-label" htmlFor="compose-to-input">To</label>
        <ContactAutocomplete
          id="compose-to-input"
          name="to"
          ariaLabelledBy="to-label"
          value={[]}
          onChange={vi.fn()}
          accountId="account-1"
          placeholder="recipient@example.com"
        />
      </>,
    );

    const input = screen.getByRole("combobox", { name: "To" });
    expect(input.getAttribute("id")).toBe("compose-to-input");
    expect(input.getAttribute("name")).toBe("to");
    expect(input.getAttribute("aria-labelledby")).toBe("to-label");
    expect(input.getAttribute("autocomplete")).toBe("email");
  });

  it("can expose its pending text as controlled input state", () => {
    const onInputValueChange = vi.fn();
    const { rerender } = render(
      <>
        <label id="to-label" htmlFor="compose-to-input">To</label>
        <ContactAutocomplete
          id="compose-to-input"
          ariaLabelledBy="to-label"
          value={[]}
          onChange={vi.fn()}
          accountId="account-1"
          inputValue=""
          onInputValueChange={onInputValueChange}
        />
      </>,
    );

    const input = screen.getByRole("combobox", { name: "To" }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "typed@example.com" } });
    expect(onInputValueChange).toHaveBeenCalledWith("typed@example.com");

    rerender(
      <>
        <label id="to-label" htmlFor="compose-to-input">To</label>
        <ContactAutocomplete
          id="compose-to-input"
          ariaLabelledBy="to-label"
          value={[]}
          onChange={vi.fn()}
          accountId="account-1"
          inputValue="typed@example.com"
          onInputValueChange={onInputValueChange}
        />
      </>,
    );

    expect(input.value).toBe("typed@example.com");
  });

  it("shows saved and recent sources and selects only the address", async () => {
    searchContactSuggestionsMock.mockResolvedValue([
      suggestion({
        contact_id: "contact-1",
        name: "Alice",
        address: "alice@example.com",
        source: "saved",
        is_favorite: true,
        last_interaction_at: null,
      }),
      suggestion({ name: "Alex", address: "alex@example.com", last_interaction_at: 100 }),
    ]);
    const onChange = vi.fn();
    render(<ContactAutocomplete value={[]} onChange={onChange} accountId="account-1" />);

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "al" } });

    expect(await screen.findByText("Saved contact")).toBeTruthy();
    expect(screen.getByText("Recent")).toBeTruthy();
    fireEvent.click(screen.getByRole("option", { name: /Alice.*alice@example.com/i }));
    expect(onChange).toHaveBeenCalledWith(["alice@example.com"]);
  });

  it("filters selected addresses without case sensitivity and supports keyboard selection", async () => {
    searchContactSuggestionsMock.mockResolvedValue([
      suggestion({
        contact_id: "contact-1",
        name: "Alice",
        address: "ALICE@example.com",
        source: "saved",
        last_interaction_at: null,
      }),
      suggestion({ name: "Bob", address: "bob@example.com", last_interaction_at: 50 }),
    ]);
    const onChange = vi.fn();
    render(
      <ContactAutocomplete
        value={["alice@example.com"]}
        onChange={onChange}
        accountId="account-1"
      />,
    );

    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "example" } });

    await waitFor(() => expect(searchContactSuggestions).toHaveBeenCalled());
    expect(screen.queryByText("ALICE@example.com")).toBeNull();
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith(["alice@example.com", "bob@example.com"]);
  });

  it("removes a recent suggestion without selecting it", async () => {
    searchContactSuggestionsMock.mockResolvedValue([
      suggestion({ name: "Alex", address: "alex@example.com", last_interaction_at: 100 }),
    ]);
    vi.mocked(suppressContactSuggestion).mockResolvedValue(undefined);
    const onChange = vi.fn();
    render(<ContactAutocomplete value={[]} onChange={onChange} accountId="account-1" />);

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "alex" } });
    const removeButton = await screen.findByRole("button", {
      name: "Remove suggestion: alex@example.com",
    });
    fireEvent.click(removeButton);

    await waitFor(() => {
      expect(suppressContactSuggestion).toHaveBeenCalledWith("alex@example.com");
      expect(screen.queryByText("alex@example.com")).toBeNull();
    });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("keeps the option mounted through pointer focus handling before selection", async () => {
    searchContactSuggestionsMock.mockResolvedValue([suggestion({ name: "Alice", address: "alice@example.com" })]);
    const onChange = vi.fn();
    render(<ContactAutocomplete value={[]} onChange={onChange} accountId="account-1" />);
    const input = screen.getByRole("combobox");
    act(() => input.focus());
    fireEvent.change(input, { target: { value: "alice" } });
    const option = await screen.findByRole("option");
    // Model the browser's default blur when clicking a non-focusable option.
    if (fireEvent.mouseDown(option)) act(() => input.blur());
    expect(option.isConnected).toBe(true);
    fireEvent.mouseUp(option);
    fireEvent.click(option);
    expect(onChange).toHaveBeenCalledWith(["alice@example.com"]);
  });

  it("keeps recent removal reachable by Tab and closes only when focus leaves", async () => {
    searchContactSuggestionsMock.mockResolvedValue([
      suggestion({ name: "Alex", address: "alex@example.com", last_interaction_at: 100 }),
    ]);
    render(<ContactAutocomplete value={[]} onChange={vi.fn()} accountId="account-1" />);

    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "alex" } });
    const removeButton = await screen.findByRole("button", {
      name: "Remove suggestion: alex@example.com",
    });

    expect(removeButton.closest('[role="option"]')).toBeNull();
    expect(fireEvent.keyDown(input, { key: "Tab" })).toBe(true);
    expect(removeButton.isConnected).toBe(true);
    act(() => removeButton.focus());
    expect(document.activeElement).toBe(removeButton);
    expect(input.getAttribute("aria-expanded")).toBe("true");
    fireEvent.blur(removeButton, { relatedTarget: document.body });
    expect(screen.queryByRole("button", { name: "Remove suggestion: alex@example.com" })).toBeNull();
  });

  it("ignores an older search response that resolves after the latest query", async () => {
    vi.useFakeTimers();
    const older = deferred<ContactSuggestion[]>();
    const latest = deferred<ContactSuggestion[]>();
    searchContactSuggestionsMock
      .mockReturnValueOnce(older.promise)
      .mockReturnValueOnce(latest.promise);

    render(
      <ContactAutocomplete
        value={[]}
        onChange={vi.fn()}
        accountId="account-1"
        placeholder="recipient@example.com"
      />,
    );
    const input = screen.getByRole("combobox");

    fireEvent.change(input, { target: { value: "old" } });
    act(() => vi.advanceTimersByTime(200));
    fireEvent.change(input, { target: { value: "new" } });
    act(() => vi.advanceTimersByTime(200));

    await act(async () => {
      latest.resolve([suggestion({ name: "Newest", address: "new@example.com" })]);
      await latest.promise;
    });
    expect(screen.getByRole("option").textContent).toContain("new@example.com");

    await act(async () => {
      older.resolve([suggestion({ name: "Outdated", address: "old@example.com" })]);
      await older.promise;
    });

    expect(screen.getByRole("option").textContent).toContain("new@example.com");
    expect(screen.getByRole("option").textContent).not.toContain("old@example.com");
  });

  it("ignores a pending response after switching accounts", async () => {
    vi.useFakeTimers();
    const oldAccountSearch = deferred<ContactSuggestion[]>();
    searchContactSuggestionsMock.mockReturnValueOnce(oldAccountSearch.promise);

    const { rerender } = render(
      <ContactAutocomplete value={[]} onChange={vi.fn()} accountId="account-1" />,
    );
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "alice" } });
    act(() => vi.advanceTimersByTime(200));

    rerender(<ContactAutocomplete value={[]} onChange={vi.fn()} accountId="account-2" />);
    await act(async () => {
      oldAccountSearch.resolve([
        suggestion({ name: "Old account", address: "old@example.com" }),
      ]);
      await oldAccountSearch.promise;
    });

    expect(screen.queryByRole("option")).toBeNull();
  });

  it("filters a contact selected externally while its search is pending", async () => {
    vi.useFakeTimers();
    const pendingSearch = deferred<ContactSuggestion[]>();
    searchContactSuggestionsMock.mockReturnValueOnce(pendingSearch.promise);

    const { rerender } = render(
      <ContactAutocomplete value={[]} onChange={vi.fn()} accountId="account-1" />,
    );
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "alice" } });
    act(() => vi.advanceTimersByTime(200));

    rerender(
      <ContactAutocomplete
        value={["alice@example.com"]}
        onChange={vi.fn()}
        accountId="account-1"
      />,
    );
    await act(async () => {
      pendingSearch.resolve([suggestion({ name: "Alice", address: "ALICE@example.com" })]);
      await pendingSearch.promise;
    });

    expect(screen.queryByRole("option")).toBeNull();
  });
});
