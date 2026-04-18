import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function readLocale(locale: string) {
  const localePath = resolve(process.cwd(), "src", "locales", `${locale}.json`);
  return JSON.parse(readFileSync(localePath, "utf8"));
}

describe("locale files", () => {
  it("translates folder count settings in English and Chinese", () => {
    const en = readLocale("en");
    const zh = readLocale("zh");

    expect(en.settings.folderCounts).toBe("Folder Counts");
    expect(en.settings.showUnreadCount).toBe("Show unread count badges in sidebar");
    expect(zh.settings.folderCounts).toBe("文件夹计数");
    expect(zh.settings.showUnreadCount).toBe("在侧边栏显示未读数徽章");
  });
});
