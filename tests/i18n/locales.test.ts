import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function readLocale(locale: string) {
  const localePath = resolve(process.cwd(), "src", "locales", `${locale}.json`);
  return JSON.parse(readFileSync(localePath, "utf8"));
}

describe("locale files", () => {
  it("translates tray menu labels in English and Chinese", () => {
    const en = readLocale("en");
    const zh = readLocale("zh");

    expect(en.tray.show).toBe("Show Window");
    expect(en.tray.hide).toBe("Hide Window");
    expect(en.tray.quit).toBe("Quit Pebble");
    expect(zh.tray.show).toBe("显示窗口");
    expect(zh.tray.hide).toBe("隐藏窗口");
    expect(zh.tray.quit).toBe("退出 Pebble");
  });

  it("translates folder count settings in English and Chinese", () => {
    const en = readLocale("en");
    const zh = readLocale("zh");

    expect(en.settings.folderCounts).toBe("Folder Counts");
    expect(en.settings.showUnreadCount).toBe("Show unread count badges in sidebar");
    expect(zh.settings.folderCounts).toBe("文件夹计数");
    expect(zh.settings.showUnreadCount).toBe("在侧边栏显示未读数徽章");
  });

  it("translates selected-text actions in English and Chinese", () => {
    const en = readLocale("en");
    const zh = readLocale("zh");

    expect(en.kanban.note).toBe("Kanban note");
    expect(en.kanban.contextNoteAdded).toBe("Added selected text to Kanban note");
    expect(en.kanban.contextNoteFailed).toBe("Failed to add Kanban note");
    expect(en.rules.contextRuleName).toBe("Selected text rule");
    expect(en.selection.actions).toBe("Selected text actions");
    expect(en.selection.copySelectedText).toBe("Copy selected text");
    expect(en.selection.copiedSelectedText).toBe("Copied selected text");
    expect(en.selection.moreActions).toBe("More selected-text actions");
    expect(en.selection.translate).toBe("Translate");
    expect(en.selection.translateSelectedText).toBe("Translate selected text");
    expect(en.selection.search).toBe("Search");
    expect(en.selection.searchSelectedText).toBe("Search selected text");
    expect(en.selection.createRule).toBe("Create rule");
    expect(en.selection.createRuleFromSelection).toBe("Create rule from selected text");
    expect(en.selection.addToKanbanNoteLabel).toBe("Add to Kanban note");
    expect(en.selection.addToKanbanNote).toBe("Add selected text as kanban note");
    expect(zh.kanban.note).toBe("看板备注");
    expect(zh.kanban.contextNoteAdded).toBe("已把选中文本加入看板备注");
    expect(zh.kanban.contextNoteFailed).toBe("加入看板备注失败");
    expect(zh.rules.contextRuleName).toBe("选中文本规则");
    expect(zh.selection.actions).toBe("选中文本操作");
    expect(zh.selection.copySelectedText).toBe("复制选中文本");
    expect(zh.selection.copiedSelectedText).toBe("已复制选中文本");
    expect(zh.selection.moreActions).toBe("更多选中文本操作");
    expect(zh.selection.translate).toBe("翻译");
    expect(zh.selection.translateSelectedText).toBe("翻译选中文本");
    expect(zh.selection.search).toBe("搜索");
    expect(zh.selection.searchSelectedText).toBe("搜索选中文本");
    expect(zh.selection.createRule).toBe("创建规则");
    expect(zh.selection.createRuleFromSelection).toBe("用选中文本创建规则");
    expect(zh.selection.addToKanbanNoteLabel).toBe("加入看板备注");
    expect(zh.selection.addToKanbanNote).toBe("加入看板备注");
  });

  it("translates remote write status labels in English and Chinese", () => {
    const en = readLocale("en");
    const zh = readLocale("zh");

    expect(en.status.remoteWritesRetrying).toBe("{{count}} remote writes retrying");
    expect(en.status.remoteWritesPending).toBe("{{count}} remote writes pending");
    expect(en.status.remoteWritesQueued).toBe("{{count}} remote writes queued");
    expect(zh.status.remoteWritesRetrying).toBeTruthy();
    expect(zh.status.remoteWritesPending).toBeTruthy();
    expect(zh.status.remoteWritesQueued).toBeTruthy();
    expect(zh.status.remoteWritesRetrying).not.toBe(en.status.remoteWritesRetrying);
    expect(zh.status.remoteWritesPending).not.toBe(en.status.remoteWritesPending);
    expect(zh.status.remoteWritesQueued).not.toBe(en.status.remoteWritesQueued);
  });

  it("translates privacy settings failure and off-mode labels in English and Chinese", () => {
    const en = readLocale("en");
    const zh = readLocale("zh");

    expect(en.privacy.loadTrustedFailed).toBe("Failed to load trusted senders");
    expect(en.privacy.removeTrustFailed).toBe("Failed to remove trusted sender");
    expect(en.privacy.trackerBlockingDesc).toBe(
      "Known tracking pixels and tracker domains are blocked unless privacy is Off or the sender is fully trusted.",
    );
    expect(en.privacy.trackerBlockingOff).toBe(
      "Tracker blocking is disabled in Off mode. All images and trackers are loaded directly.",
    );
    expect(zh.privacy.loadTrustedFailed).toBeTruthy();
    expect(zh.privacy.removeTrustFailed).toBeTruthy();
    expect(zh.privacy.trackerBlockingDesc).toBeTruthy();
    expect(zh.privacy.trackerBlockingOff).toBeTruthy();
    expect(zh.privacy.loadTrustedFailed).not.toBe(en.privacy.loadTrustedFailed);
    expect(zh.privacy.removeTrustFailed).not.toBe(en.privacy.removeTrustFailed);
    expect(zh.privacy.trackerBlockingDesc).not.toBe(en.privacy.trackerBlockingDesc);
    expect(zh.privacy.trackerBlockingOff).not.toBe(en.privacy.trackerBlockingOff);
  });
});
