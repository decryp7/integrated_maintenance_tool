/** One stop of the guided tour. */
export interface TourStep {
  id: string;
  title: string;
  text: string;
  /** View to open (DockService.show) before looking for the target; none for a centred step. */
  view?: string;
  /** CSS selector of the element to spotlight; none for a centred step. */
  target?: string;
}

/** The tour of the demo incident: what each part of the screen is for. */
export const TOUR_STEPS: readonly TourStep[] = [
  {
    id: 'welcome',
    title: 'A short tour',
    text: 'This demo is a real-looking incident: an order service crashed on one machine while others kept running. Six quick stops show where to look. Use the arrow keys, or press Escape to leave at any time.',
  },
  {
    id: 'timeline',
    title: 'Timeline',
    text: 'Each lane is one running instance (machine and process). Coloured marks are errors, crashes and restarts. Drag across the timeline to zoom into a moment.',
    view: 'timeline',
    target: 'imt-timeline-panel',
  },
  {
    id: 'events',
    title: 'Events',
    text: 'Every event that matches your filters, in time order. Click one to see all of its properties in the inspector; right-click for actions.',
    view: 'events',
    target: 'imt-events-grid',
  },
  {
    id: 'query',
    title: 'Query bar',
    text: 'Ask precise questions, like level >= error and MachineName = "WEB01". It completes field names as you type and points at mistakes.',
    view: 'query',
    target: 'imt-query-view',
  },
  {
    id: 'moment',
    title: 'Moment',
    text: 'What was happening around a point in time, across every source: errors, starts and stops, slow requests. Pick a time on the timeline to fill it.',
    view: 'moment',
    target: 'imt-moment-view',
  },
  {
    id: 'markers',
    title: 'Markers',
    text: 'Markers label important events (crashes, restarts, deployments) on the timeline. Add your own rules here; they are saved in your profile.',
    view: 'markers',
    target: 'imt-markers-manager',
  },
  {
    id: 'overview',
    title: 'Overview',
    text: 'The health check: what went wrong, most severe first, each with a button that takes you there. It opens after every import. That is the tour: enjoy exploring!',
    view: 'overview',
    target: 'imt-overview-view',
  },
];
