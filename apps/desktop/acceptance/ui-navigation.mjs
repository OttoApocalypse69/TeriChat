// Shared browser-driver actions for the UI's default-closed details panel.
export async function openWorkspaceDetails(page) {
  const details = page.getByRole('complementary', { name: 'Workspace details', exact: true });
  if (!await details.isVisible()) {
    await page.getByRole('button', { name: 'Workspace details', exact: true }).first().click();
  }
  await details.waitFor();
  return details;
}
