import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import MessageParticipants from "../../src/components/MessageParticipants";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: unknown) => {
      if (typeof options === "string") return options;
      const opts = (options ?? {}) as Record<string, string | number>;
      const template = typeof opts.defaultValue === "string" ? opts.defaultValue : key;
      return template.replace(/\{\{(\w+)\}\}/g, (_match, name) => String(opts[name] ?? ""));
    },
  }),
}));

vi.mock("../../src/components/ContactAddressAction", () => ({
  default: ({ address }: { address: string }) => (
    <span data-testid="contact-address-action">{address}</span>
  ),
}));

function makeParticipants(count: number, prefix: string) {
  return Array.from({ length: count }, (_, index) => ({
    name: null,
    address: `${prefix}${index}@example.com`,
  }));
}

function groupsFor(to: ReturnType<typeof makeParticipants>, cc: ReturnType<typeof makeParticipants> = []) {
  return [
    { label: "To:", participants: to },
    { label: "Cc:", participants: cc },
  ];
}

describe("MessageParticipants", () => {
  it("collapses large recipient lists behind a more toggle", () => {
    render(<MessageParticipants accountId="account-1" groups={groupsFor(makeParticipants(10, "to"))} />);

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(3);
    expect(screen.getByRole("button", { name: "+7 more" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "+7 more" }));

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(10);
    expect(screen.getByRole("button", { name: "Show less" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Show less" }));

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(3);
    expect(screen.getByRole("button", { name: "+7 more" })).toBeTruthy();
  });

  it("spreads the collapsed budget across To and Cc groups", () => {
    render(
      <MessageParticipants
        accountId="account-1"
        groups={groupsFor(makeParticipants(2, "to"), makeParticipants(3, "cc"))}
      />,
    );

    expect(screen.getAllByTestId("contact-address-action").map((node) => node.textContent)).toEqual([
      "to0@example.com",
      "to1@example.com",
      "cc0@example.com",
    ]);
    expect(screen.getByRole("button", { name: "+2 more" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "+2 more" }));

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(5);
  });

  it("caps the expanded list height and keeps the collapse toggle outside the scroll area", () => {
    render(<MessageParticipants accountId="account-1" groups={groupsFor(makeParticipants(10, "to"))} />);

    const groupsNode = document.querySelector(".message-participants-groups");
    expect(groupsNode).not.toBeNull();
    expect(groupsNode?.className).not.toContain("message-participants-groups--scroll");

    fireEvent.click(screen.getByRole("button", { name: "+7 more" }));

    expect(groupsNode?.className).toContain("message-participants-groups--scroll");
    const showLess = screen.getByRole("button", { name: "Show less" });
    expect(groupsNode?.contains(showLess)).toBe(false);
  });

  it("does not collapse when participants fit the limit", () => {
    render(
      <MessageParticipants
        accountId="account-1"
        groups={groupsFor(makeParticipants(2, "to"), makeParticipants(1, "cc"))}
      />,
    );

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(3);
    expect(screen.queryByRole("button", { name: /\+|less/i })).toBeNull();
  });

  it("pairs each chip with its own inline contact action", () => {
    render(
      <MessageParticipants
        accountId="account-1"
        groups={[{ label: "To:", participants: [{ name: "Alice", address: "alice@example.com" }] }]}
      />,
    );

    const chips = document.querySelectorAll(".participant-chip");
    expect(chips).toHaveLength(1);
    expect(chips[0].querySelector("[data-testid='contact-address-action']")).not.toBeNull();
    expect(chips[0].textContent).toContain("Alice");
  });

  it("skips empty groups and duplicate addresses", () => {
    render(
      <MessageParticipants
        accountId="account-1"
        groups={[
          { label: "To:", participants: [
            { name: null, address: "alice@example.com" },
            { name: "Alice Duplicate", address: "alice@example.com" },
          ] },
          { label: "Cc:", participants: [] },
        ]}
      />,
    );

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(1);
    expect(screen.queryByText("Cc:")).toBeNull();
  });
});
