import { type Browser, type Page, expect, test } from "@playwright/test";

/**
 * The complete Hourwise workflow, as real users in a real browser, against the isolated
 * stack from docker-compose.e2e.yml (fresh database every run).
 */

const ADMIN = { email: "e2e-admin@example.com", password: "E2eAdminPass1" };
const stamp = Date.now();
const EMPLOYEE = {
  name: "Erin Employee",
  email: `erin.${stamp}@example.com`,
  password: "ErinPass123",
};
const PROJECT = `Website relaunch ${stamp}`;

test.describe.configure({ mode: "serial" });

async function login(page: Page, email: string, password: string) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
}

async function newPage(browser: Browser) {
  const context = await browser.newContext();
  return context.newPage();
}

test("admin sets up an employee and a project; employee tracks time; admin reviews", async ({
  browser,
}) => {
  // ---- Admin logs in
  const admin = await newPage(browser);
  await login(admin, ADMIN.email, ADMIN.password);
  await expect(admin).toHaveURL(/\/admin$/);
  await expect(admin.getByText("Welcome back, E2E")).toBeVisible();

  // Session cookie is HttpOnly and nothing is kept in localStorage.
  const cookies = await admin.context().cookies();
  expect(cookies.find((c) => c.name === "hw_session")?.httpOnly).toBe(true);
  expect(await admin.evaluate(() => window.localStorage.length)).toBe(0);

  // ---- Admin creates an employee
  await admin.getByRole("link", { name: "Team" }).click();
  await admin.getByRole("button", { name: "Add person" }).click();
  const userDialog = admin.getByRole("dialog");
  await userDialog.getByLabel(/^Full name/).fill(EMPLOYEE.name);
  await userDialog.getByLabel(/^Email/).fill(EMPLOYEE.email);
  await userDialog.getByLabel(/^Initial password/).fill(EMPLOYEE.password);
  await userDialog.getByRole("button", { name: "Add person" }).click();
  await expect(userDialog).toBeHidden();
  await expect(admin.getByText(EMPLOYEE.email)).toBeVisible();

  // ---- Admin creates a project and assigns the employee
  await admin.getByRole("link", { name: "Projects" }).click();
  await admin.getByRole("button", { name: /New project/ }).click();
  const projectDialog = admin.getByRole("dialog");
  await projectDialog.getByLabel(/^Project name/).fill(PROJECT);
  await projectDialog.getByLabel(/^Client/).fill("ACME");
  await projectDialog.getByRole("button", { name: "Create project" }).click();
  await expect(projectDialog).toBeHidden();
  await admin.getByRole("link", { name: PROJECT }).click();
  await admin.getByRole("button", { name: /Add members/ }).click();
  const membersDialog = admin.getByRole("dialog");
  await membersDialog.getByLabel(new RegExp(EMPLOYEE.name)).check();
  await membersDialog.getByRole("button", { name: "Add 1 member" }).click();
  await expect(membersDialog).toBeHidden();
  await expect(admin.getByText(EMPLOYEE.email)).toBeVisible();

  // ---- Employee logs in
  const employee = await newPage(browser);
  await login(employee, EMPLOYEE.email, EMPLOYEE.password);
  await expect(employee).toHaveURL(/\/time$/);
  await expect(employee.getByText("No time tracked for this day")).toBeVisible();

  // ---- Employee records work hours
  await employee.getByRole("button", { name: "Track time" }).click();
  const entryDialog = employee.getByRole("dialog");
  await expect(entryDialog.getByLabel("Project")).toHaveValue(/.+/);
  await entryDialog.getByLabel("Hours").fill("2");
  await entryDialog.getByLabel("Minutes").fill("30");
  await entryDialog.getByLabel(/^Notes/).fill("Built the new landing page");
  await entryDialog.getByRole("button", { name: "Save entry" }).click();
  await expect(entryDialog).toBeHidden();
  await expect(employee.getByText("Built the new landing page")).toBeVisible();

  // ---- Employee sees the correct day and weekly total
  // The week summary cell (not the Day/Week toggle button).
  const weekCell = employee
    .locator("div", { has: employee.locator("div", { hasText: /^Week$/ }) })
    .last();
  await expect(weekCell).toContainText("2:30");
  await expect(employee.getByText("Total", { exact: true }).locator("..")).toContainText("2:30");

  // The 24h rule is enforced end to end.
  await employee.getByRole("button", { name: "Track time" }).click();
  await entryDialog.getByLabel("Hours").fill("22");
  await entryDialog.getByRole("button", { name: "Save entry" }).click();
  await expect(entryDialog.getByRole("alert")).toContainText("24 hours");
  await entryDialog.getByRole("button", { name: "Cancel" }).click();

  // ---- Admin reviews the submitted hours
  await admin.reload();
  await expect(admin.getByText("Total time").locator("..")).toContainText("2:30");
  await admin.getByRole("button", { name: "Time & work" }).click();
  await expect(admin.getByText("Built the new landing page")).toBeVisible();

  // ---- Employee updates the entry
  await employee.getByRole("button", { name: "Edit entry" }).click();
  await entryDialog.getByLabel("Hours").fill("3");
  await entryDialog.getByLabel("Minutes").fill("15");
  await entryDialog
    .getByLabel(/^Notes/)
    .fill("Built the new landing page and fixed review comments");
  await entryDialog.getByRole("button", { name: "Update entry" }).click();
  await expect(entryDialog).toBeHidden();
  await expect(weekCell).toContainText("3:15");

  // ---- Reports display the correct information
  await employee.getByRole("link", { name: "Reports" }).click();
  await expect(employee.getByText("3h 15m").first()).toBeVisible();
  await expect(employee.getByText("3.25 hours")).toBeVisible();
  await expect(
    employee.getByText("Built the new landing page and fixed review comments"),
  ).toBeVisible();

  await admin.getByRole("link", { name: "Reports" }).click();
  await expect(
    admin.getByText("Built the new landing page and fixed review comments"),
  ).toBeVisible();

  // ---- CSV export downloads the report
  const downloadPromise = admin.waitForEvent("download");
  await admin.getByRole("button", { name: /Export CSV/ }).click();
  const download = await downloadPromise;
  const stream = await download.createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(chunk as Buffer);
  const csv = Buffer.concat(chunks).toString("utf8");
  expect(csv).toContain(
    `"${EMPLOYEE.name}","${EMPLOYEE.email}","ACME","${PROJECT}","3","15","3.25"`,
  );

  // ---- Employee cannot reach the admin interface
  await employee.goto("/admin");
  await expect(employee).not.toHaveURL(/\/admin/);
  const forbidden = await employee.request.get("/api/admin/users");
  expect(forbidden.status()).toBe(403);

  // ---- Employee logs out
  await employee.goto("/time");
  await employee.getByRole("button", { name: "Sign out" }).click();
  await expect(employee).toHaveURL(/\/login$/);
  await employee.goto("/time");
  await expect(employee).toHaveURL(/\/login$/);
  const afterLogout = await employee.request.get("/api/me");
  expect(afterLogout.status()).toBe(401);
});

test("wrong password shows an error and does not sign in", async ({ page }) => {
  await login(page, ADMIN.email, "definitely-wrong");
  await expect(page.getByRole("alert").filter({ hasText: "Invalid credentials" })).toBeVisible();
  await expect(page).toHaveURL(/\/login$/);
});
