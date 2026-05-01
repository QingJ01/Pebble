import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import ContactAutocomplete from "../../src/components/ContactAutocomplete";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback?: string) => fallback ?? _key }),
}));

vi.mock("../../src/lib/api", () => ({
  searchContacts: vi.fn(),
}));

vi.mock("../../src/stores/toast.store", () => ({
  useToastStore: { getState: () => ({ addToast: vi.fn() }) },
}));

describe("ContactAutocomplete", () => {
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
});
