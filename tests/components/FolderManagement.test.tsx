import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Account, Folder } from "../../src/lib/api";
import { listFolders } from "../../src/lib/api";
import { createMailFolder, renameMailFolder, getMailMoveContext, moveMailMessages, getMailFolderParents } from "../../src/lib/folders-api";
import FolderManagementButton from "../../src/components/FolderManagementButton";
import MoveToButton from "../../src/components/MoveToButton";
import { useMailStore } from "../../src/stores/mail.store";
import { folderTree } from "../../src/lib/folderManagement";
import en from "../../src/locales/folders.en.json";
import zh from "../../src/locales/folders.zh.json";

vi.mock("../../src/lib/api", () => ({ listFolders: vi.fn() }));
vi.mock("../../src/lib/folders-api", () => ({ createMailFolder: vi.fn(), renameMailFolder: vi.fn(), renameMailFolderNamespace:vi.fn(), getMailMoveContext: vi.fn(), moveMailMessages: vi.fn(), getMailFolderParents: vi.fn() }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string, options?: string | Record<string, unknown>) => {
  if (typeof options === "string") return options;
  if (options?.defaultValue) return String(options.defaultValue).replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(options[name]));
  return key;
} }) }));

const account: Account = {id:"a",email:"test@example.invalid",display_name:"Test",account_label:null,provider_display_name:null,color:null,provider:"imap",created_at:1,updated_at:1};
const folder = (id: string, overrides: Partial<Folder> = {}): Folder => ({id,account_id:"a",remote_id:id,name:id,folder_type:"folder",role:null,parent_id:null,color:null,is_system:false,sort_order:0,...overrides});
const source=folder("source"), target=folder("target"), keep=folder("keep");
const messages=[{id:"m1",account_id:"a",is_draft:false,thread_id:"t1"},{id:"m2",account_id:"a",is_draft:false,thread_id:"t1"}];
let client: QueryClient;
function mount(element: React.ReactNode) {return render(<QueryClientProvider client={client}>{element}</QueryClientProvider>);}
beforeEach(() => {
  vi.clearAllMocks();client=new QueryClient({defaultOptions:{queries:{retry:false,gcTime:0}}});
  useMailStore.setState({activeFolderId:"source",selectedMessageId:"m2",selectedThreadId:"t1",selectedMessageIds:new Set(["m1","m2"])});
  vi.mocked(listFolders).mockResolvedValue([source,target]);
  vi.mocked(getMailFolderParents).mockResolvedValue([]);
  vi.mocked(getMailMoveContext).mockResolvedValue({provider:"imap",folder_ids_by_message:{m1:["source"],m2:["source"]}});
  vi.mocked(moveMailMessages).mockResolvedValue({moved_ids:["m1","m2"],failures:[]});
});
afterEach(() => {cleanup();client.clear();});

describe("folder management", () => {
  it("creates beneath the selected account/parent and closes only after success", async () => {
    vi.mocked(createMailFolder).mockResolvedValue(folder("created"));const invalidation=vi.spyOn(client,"invalidateQueries");
    mount(<FolderManagementButton accounts={[{...account,provider:"pop3"}]} folders={[source]} accountId="a"/>);
    fireEvent.click(screen.getByRole("button",{name:"Manage folders"}));
    expect(screen.getByText("POP3 folders and moves are stored on this device only.")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Parent folder"),{target:{value:"source"}});fireEvent.change(screen.getByLabelText("Name"),{target:{value:"新文件夹"}});
    fireEvent.click(screen.getByRole("button",{name:"Save"}));
    await waitFor(() => expect(createMailFolder).toHaveBeenCalledWith("a","新文件夹","source"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());expect(invalidation).toHaveBeenCalledWith({queryKey:["folders"]});
  });
  it("keeps the dialog and server error visible when creation fails", async () => {
    vi.mocked(createMailFolder).mockRejectedValue(new Error("Authentication expired"));
    mount(<FolderManagementButton accounts={[account]} folders={[]} accountId="a"/>);fireEvent.click(screen.getByRole("button",{name:"Manage folders"}));
    fireEvent.change(screen.getByLabelText("Name"),{target:{value:"New"}});fireEvent.click(screen.getByRole("button",{name:"Save"}));
    expect((await screen.findByRole("alert")).textContent).toContain("Authentication expired");expect(screen.getByRole("dialog")).toBeTruthy();
  });
  it("protects system folders and explains Gmail parent-label rename", () => {
    const inbox=folder("inbox",{is_system:true,role:"inbox"}), parent=folder("parent",{name:"Parent"}), child=folder("child",{name:"Parent/Child",parent_id:"parent"});
    mount(<FolderManagementButton accounts={[{...account,provider:"gmail"}]} folders={[inbox,parent,child]} accountId="a"/>);fireEvent.click(screen.getByRole("button",{name:"Manage folders"}));
    fireEvent.change(screen.getByLabelText("Action"),{target:{value:"rename"}});fireEvent.change(screen.getByLabelText("Folder"),{target:{value:"inbox"}});
    expect((screen.getByRole("button",{name:"Save"}) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Folder"),{target:{value:"parent"}});expect(screen.getByText("folderManagement.gmailParentRename")).toBeTruthy();expect(renameMailFolder).not.toHaveBeenCalled();
  });
  it("restores keyboard focus when Escape closes the modal", () => {
    mount(<FolderManagementButton accounts={[account]} folders={[]} accountId="a"/>);const trigger=screen.getByRole("button",{name:"Manage folders"});trigger.focus();fireEvent.click(trigger);
    expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true);fireEvent.keyDown(document,{key:"Escape"});
    expect(screen.queryByRole("dialog")).toBeNull();expect(document.activeElement).toBe(trigger);
  });
  it("keeps English and Chinese feature keys complete", () => {expect(Object.keys(en.folderManagement).sort()).toEqual(Object.keys(zh.folderManagement).sort());expect(Object.values(zh.folderManagement).every(Boolean)).toBe(true);});
  it("offers nonselectable IMAP namespace parents and submits the server remote name", async () => {
    vi.mocked(getMailFolderParents).mockResolvedValue([{remote_id:"Container",name:"Container",delimiter:".",selectable:false,is_system:false}]);vi.mocked(createMailFolder).mockResolvedValue(folder("created"));
    mount(<FolderManagementButton accounts={[account]} folders={[]} accountId="a"/>);fireEvent.click(screen.getByRole("button",{name:"Manage folders"}));await screen.findByRole("option",{name:"Container"});
    fireEvent.change(screen.getByLabelText("Parent folder"),{target:{value:"Container"}});fireEvent.change(screen.getByLabelText("Name"),{target:{value:"New"}});fireEvent.click(screen.getByRole("button",{name:"Save"}));
    await waitFor(()=>expect(createMailFolder).toHaveBeenCalledWith("a","New",null,"Container"));
  });
  it("loads the chosen account even when the classic sidebar supplied only another account's folders", async () => {
    const second={...account,id:"b",email:"second@example.invalid",provider:"pop3" as const};const secondFolder=folder("b-folder",{account_id:"b",name:"B folder"});
    vi.mocked(listFolders).mockImplementation(async(id)=>id==="b"?[secondFolder]:[source]);vi.mocked(renameMailFolder).mockResolvedValue([secondFolder]);
    mount(<FolderManagementButton accounts={[account,second]} folders={[source]} accountId="a"/>);fireEvent.click(screen.getByRole("button",{name:"Manage folders"}));fireEvent.change(screen.getByLabelText("Account"),{target:{value:"b"}});fireEvent.change(screen.getByLabelText("Action"),{target:{value:"rename"}});
    await screen.findByRole("option",{name:"B folder"});fireEvent.change(screen.getByLabelText("Folder"),{target:{value:"b-folder"}});fireEvent.change(screen.getByLabelText("Name"),{target:{value:"New B"}});fireEvent.click(screen.getByRole("button",{name:"Save"}));
    await waitFor(()=>expect(renameMailFolder).toHaveBeenCalledWith("b","b-folder","New B"));
  });
  it("builds hierarchical Graph paths and terminates corrupt cycles", () => {
    const parent=folder("p",{remote_id:"opaque-p",name:"Projects"}), child=folder("c",{remote_id:"opaque-c",name:"One",parent_id:"p"});
    expect(folderTree([child,parent]).find((f)=>f.folder.id==="c")).toMatchObject({depth:1,path:"Projects / One"});
    expect(folderTree([{...parent,parent_id:"c"},child])).toHaveLength(2);
  });
});

describe("Move To", () => {
  it.each(["t1", null])("closes a successfully moved thread detail with thread ID %s", async (threadId) => {
    const message = {...messages[0],thread_id:threadId};
    useMailStore.setState({selectedThreadId:threadId ?? message.id,selectedMessageId:message.id});
    vi.mocked(moveMailMessages).mockResolvedValue({moved_ids:[message.id],failures:[]});
    mount(<MoveToButton messages={[message]}/>);fireEvent.click(screen.getByRole("button",{name:"Move to folder"}));
    await screen.findByRole("option",{name:"target"});fireEvent.change(screen.getByLabelText("Destination"),{target:{value:"target"}});fireEvent.click(screen.getByRole("button",{name:"Move"}));
    await waitFor(()=>expect(useMailStore.getState().selectedThreadId).toBeNull());
    expect(useMailStore.getState().selectedMessageId).toBeNull();
  });
  it("does not close another thread selected while a move is pending", async () => {
    useMailStore.setState({selectedThreadId:"unrelated-thread"});
    mount(<MoveToButton messages={messages}/>);fireEvent.click(screen.getByRole("button",{name:"Move to folder"}));
    await screen.findByRole("option",{name:"target"});fireEvent.change(screen.getByLabelText("Destination"),{target:{value:"target"}});fireEvent.click(screen.getByRole("button",{name:"Move"}));
    await waitFor(()=>expect(screen.queryByRole("dialog")).toBeNull());
    expect(useMailStore.getState().selectedThreadId).toBe("unrelated-thread");
  });
  it("retains failed batch selections and details, and reports only moved IDs", async () => {
    vi.mocked(moveMailMessages).mockResolvedValue({moved_ids:["m1"],failures:[{message_id:"m2",error:"offline; queued"}]});const onMoved=vi.fn();
    mount(<MoveToButton messages={messages} onMoved={onMoved}/>);fireEvent.click(screen.getByRole("button",{name:"Move to folder"}));
    await screen.findByRole("option",{name:"target"});fireEvent.change(screen.getByLabelText("Destination"),{target:{value:"target"}});fireEvent.click(screen.getByRole("button",{name:"Move"}));
    await waitFor(()=>expect(onMoved).toHaveBeenCalledWith(["m1"]));expect(moveMailMessages).toHaveBeenCalledWith("a",["m1","m2"],"source","target");
    expect([...useMailStore.getState().selectedMessageIds]).toEqual(["m2"]);expect(useMailStore.getState().selectedMessageId).toBe("m2");expect(useMailStore.getState().selectedThreadId).toBe("t1");
    expect(screen.getByRole("alert").textContent).toContain("1 moved; 1 failed");expect(screen.getByRole("dialog")).toBeTruthy();
  });
  it("requires an explicit source for ambiguous Gmail labels and keeps other labels", async () => {
    useMailStore.setState({activeFolderId:null});vi.mocked(listFolders).mockResolvedValue([source,target,keep]);
    vi.mocked(getMailMoveContext).mockResolvedValue({provider:"gmail",folder_ids_by_message:{m1:["source","keep"]}});
    mount(<MoveToButton messages={[messages[0]]}/>);fireEvent.click(screen.getByRole("button",{name:"Move to folder"}));await screen.findByText("Move removes only the selected source label. Other labels are kept.");
    expect((screen.getByRole("button",{name:"Move"}) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Source folder"),{target:{value:"keep"}});fireEvent.change(screen.getByLabelText("Destination"),{target:{value:"target"}});fireEvent.click(screen.getByRole("button",{name:"Move"}));
    await waitFor(()=>expect(moveMailMessages).toHaveBeenCalledWith("a",["m1"],"keep","target"));
  });
  it("disables current/device-only targets and excludes drafts, outbox and foreign accounts", async () => {
    vi.mocked(listFolders).mockResolvedValue([source,target,folder("local",{remote_id:"__local_archive__"}),folder("drafts",{role:"drafts"}),folder("outbox",{remote_id:"__local_outbox__"})]);
    mount(<MoveToButton messages={[messages[0]]}/>);fireEvent.click(screen.getByRole("button",{name:"Move to folder"}));
    const current=await screen.findByRole("option",{name:"source"});expect((current as HTMLOptionElement).disabled).toBe(true);
    expect((screen.getByRole("option",{name:/local \(On this device\)/}) as HTMLOptionElement).disabled).toBe(true);expect(screen.queryByRole("option",{name:"drafts"})).toBeNull();expect(screen.queryByRole("option",{name:"outbox"})).toBeNull();
  });
  it("does not invoke a move for drafts or mixed accounts", () => {
    const {rerender}=mount(<MoveToButton messages={[{...messages[0],is_draft:true}]}/>);expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
    rerender(<QueryClientProvider client={client}><MoveToButton messages={[messages[0],{...messages[1],account_id:"b"}]}/></QueryClientProvider>);expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);expect(moveMailMessages).not.toHaveBeenCalled();
  });
  it("keeps selection unchanged on network failure", async () => {
    vi.mocked(moveMailMessages).mockRejectedValue(new Error("network unavailable"));mount(<MoveToButton messages={messages}/>);fireEvent.click(screen.getByRole("button",{name:"Move to folder"}));await screen.findByRole("option",{name:"target"});
    fireEvent.change(screen.getByLabelText("Destination"),{target:{value:"target"}});fireEvent.click(screen.getByRole("button",{name:"Move"}));expect((await screen.findByRole("alert")).textContent).toContain("network unavailable");expect([...useMailStore.getState().selectedMessageIds]).toEqual(["m1","m2"]);
  });
});
