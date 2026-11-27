# Rustward console design

**Accepted design rules.** These rules guide contributors who design, build, or
review the private web console. They describe how the console presents service
behavior to the administrator. The [architecture](ARCHITECTURE.md) describes service behavior.
The [glossary](GLOSSARY.md) defines the terms used in the console.

## Interaction

Each bold statement gives a principle. The instructions that follow state its
requirements. Examples illustrate possible wording or layouts.

- **Report the outcome of each action.** After admission, show the run ID and
  current state. Show “Running” only after the program starts. For example,
  “Run 42 queued” confirms admission while the run waits for dispatch.
- **Name the action and its scope.** Identify what the control affects and
  explain its effect beside the control. For example, label the controls
  “Pause schedule” and “Pause dispatch.” Beside “Pause schedule,” the text
  “Queued runs can still start” explains the scope of the pause.
- **Put the next decision first.** In run history, show the task, run ID,
  state, and time. Label the time zone and identify runs that need review.
  Put inputs and output in run detail.
- **Use consistent controls and language.** Use the same state labels in list
  and detail views. Use sentence case for controls. Follow IBM Carbon
  patterns for tables and forms.
- **Help the administrator correct errors.** Keep entered values after a
  validation error. Identify each field that needs a correction and explain
  which values it accepts. When several fields need changes, link to each
  field from an error summary.

## Visual style

Use restrained neo-brutalism. Give the screen a clear frame and keep repeated
data easy to scan.

- Support light and dark modes on every console screen. Use warm off-white
  surfaces with near-black text in light mode and dark surfaces with light
  text in dark mode. Keep the meaning of each state color consistent.
- Use square corners in both modes. Put a
  strong outer border around a table and thin dividers between rows. Align
  values within each column and use a consistent spacing scale.
- Use bold sans serif headings and regular sans serif labels. Use monospace
  for run IDs, timestamps, and command output.
- Give one primary action visual emphasis. For example, use a blue “Submit
  run” button with a small hard offset shadow that closes when pressed.
  Keep panels and state labels flat.
- Use amber for conditions that need attention and red for failures or
  destructive actions. Keep routine states neutral. Pair each state color
  with a text label.

## Accessibility and review

Use WCAG 2.2 Level AA as the accessibility target. Check text, controls,
state labels, and focus outlines for contrast in both modes. Make controls usable with a keyboard and give keyboard focus a
visible outline. Announce action results to assistive technology.

Review the Runs screen in both modes with dispatch paused and a run in the queue. Show
“Dispatch paused. Queued runs will wait.” once above the rows. Keep “Queued”
on the affected run. Check that the administrator can identify the run, tell
why it is waiting, and open its details with a keyboard.

## References

- [IBM Carbon data table guidelines](https://carbondesignsystem.com/components/data-table/usage/)
- [Nielsen’s usability heuristics](https://www.nngroup.com/articles/ten-usability-heuristics/)
- [GOV.UK error summary guidance](https://design-system.service.gov.uk/components/error-summary/)
- [Web Content Accessibility Guidelines 2.2](https://www.w3.org/TR/WCAG22/)
