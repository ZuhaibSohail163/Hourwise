import type * as ApiModule from "@/lib/api";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { EntryModal } from "@/components/EntryModal";
import { ApiError, api } from "@/lib/api";
import { makeEntry, makeProject } from "@/test/fixtures";

vi.mock("@/lib/api", async (orig) => ({
  ...(await orig<typeof ApiModule>()),
  api: vi.fn(),
}));
const apiMock = vi.mocked(api);

const projects = [
  makeProject({ id: "p1", name: "Website", client: "ACME" }),
  makeProject({ id: "p2", name: "Mobile app", client: null }),
  makeProject({ id: "p3", name: "Old thing", is_archived: true }),
];

function setup(props: Partial<React.ComponentProps<typeof EntryModal>> = {}) {
  const onSaved = vi.fn();
  const onClose = vi.fn();
  render(
    <EntryModal
      open
      onClose={onClose}
      onSaved={onSaved}
      projects={projects}
      defaultDate="2026-11-09"
      {...props}
    />,
  );
  return { onSaved, onClose, user: userEvent.setup() };
}

describe("EntryModal", () => {
  beforeEach(() => {
    apiMock.mockReset();
  });

  it("only offers active projects", () => {
    setup();
    const options = screen.getAllByRole("option").map((o) => o.textContent);
    expect(options).toEqual(["ACME — Website", "Mobile app"]);
  });

  it("keeps an archived project selectable when editing an entry on it", () => {
    setup({ entry: makeEntry({ project_id: "p3", project_name: "Old thing" }) });
    expect(screen.getByRole("option", { name: /Old thing \(archived\)/ })).toBeInTheDocument();
  });

  it("creates an entry from hours + minutes", async () => {
    const saved = makeEntry({ minutes: 150 });
    apiMock.mockResolvedValue(saved);
    const { user, onSaved, onClose } = setup();

    await user.selectOptions(screen.getByLabelText("Project"), "p2");
    await user.type(screen.getByLabelText("Hours"), "2");
    await user.type(screen.getByLabelText("Minutes"), "30");
    await user.type(screen.getByLabelText(/Notes/), "Fixed the login bug");
    await user.click(screen.getByRole("button", { name: "Save entry" }));

    expect(apiMock).toHaveBeenCalledWith("/time-entries", {
      method: "POST",
      body: {
        project_id: "p2",
        entry_date: "2026-11-09",
        minutes: 150,
        description: "Fixed the login bug",
      },
    });
    expect(onSaved).toHaveBeenCalledWith(saved);
    expect(onClose).toHaveBeenCalled();
  });

  it("edits an existing entry with PATCH and prefilled values", async () => {
    const entry = makeEntry({ id: "e9", minutes: 95, description: "Old notes" });
    apiMock.mockResolvedValue(entry);
    const { user } = setup({ entry });
    expect(screen.getByLabelText("Hours")).toHaveValue(1);
    expect(screen.getByLabelText("Minutes")).toHaveValue(35);
    expect(screen.getByLabelText(/Notes/)).toHaveValue("Old notes");
    await user.click(screen.getByRole("button", { name: "Update entry" }));
    expect(apiMock).toHaveBeenCalledWith(
      "/time-entries/e9",
      expect.objectContaining({ method: "PATCH" }),
    );
  });

  // Out-of-range or fractional numbers are stopped by the inputs' own min/max/step constraints
  // before the form submits (the browser shows its validation bubble).
  it.each([
    ["1", "60", "Minutes"],
    ["25", "", "Hours"],
    ["-1", "", "Hours"],
    ["1.5", "", "Hours"],
  ])("blocks hours=%s minutes=%s via input constraints", async (h, m, invalidField) => {
    const { user } = setup();
    if (h) await user.type(screen.getByLabelText("Hours"), h);
    if (m) await user.type(screen.getByLabelText("Minutes"), m);
    await user.click(screen.getByRole("button", { name: "Save entry" }));
    expect(screen.getByLabelText(invalidField)).toBeInvalid();
    expect(apiMock).not.toHaveBeenCalled();
  });

  it.each([
    ["", "", "Please enter some time."],
    ["0", "0", "Please enter some time."],
    ["24", "1", "An entry cannot exceed 24 hours."],
  ])("rejects hours=%s minutes=%s", async (h, m, message) => {
    const { user } = setup();
    if (h) await user.type(screen.getByLabelText("Hours"), h);
    if (m) await user.type(screen.getByLabelText("Minutes"), m);
    await user.click(screen.getByRole("button", { name: "Save entry" }));
    expect(screen.getByRole("alert")).toHaveTextContent(message);
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("accepts exactly 24 hours", async () => {
    apiMock.mockResolvedValue(makeEntry());
    const { user } = setup();
    await user.type(screen.getByLabelText("Hours"), "24");
    await user.click(screen.getByRole("button", { name: "Save entry" }));
    expect(apiMock).toHaveBeenCalledWith(
      "/time-entries",
      expect.objectContaining({ body: expect.objectContaining({ minutes: 1440 }) }),
    );
  });

  it("shows server-side validation errors and stays open", async () => {
    apiMock.mockRejectedValue(new ApiError("Total time for a day cannot exceed 24 hours", 400));
    const { user, onClose } = setup();
    await user.type(screen.getByLabelText("Hours"), "8");
    await user.click(screen.getByRole("button", { name: "Save entry" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("cannot exceed 24 hours");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("explains when the user has no active projects", () => {
    setup({ projects: [makeProject({ is_archived: true })] });
    expect(screen.getByText(/added to any active projects/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save entry" })).not.toBeInTheDocument();
  });

  it("renders nothing when closed", () => {
    setup({ open: false });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
