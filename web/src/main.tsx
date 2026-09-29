import React from "react";
import ReactDOM from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router-dom";
import "./styles.css";
import App from "./App";
import HomePage from "./pages/HomePage";
import PortfolioPage from "./pages/PortfolioPage";
import BoardPage from "./pages/BoardPage";
import StatusPage from "./pages/StatusPage";
import FeaturePage from "./pages/FeaturePage";
import MilestonesPage from "./pages/MilestonesPage";
import MilestonePage from "./pages/MilestonePage";
import SchedulePage from "./pages/SchedulePage";
import GanttPage from "./pages/GanttPage";
import CharterPage from "./pages/CharterPage";
import ReviewPage from "./pages/ReviewPage";
import WorkflowPage from "./pages/WorkflowPage";
import DocsPage from "./pages/DocsPage";
import DocPage from "./pages/DocPage";
import ReleasesPage from "./pages/ReleasesPage";

const router = createBrowserRouter([
  {
    path: "/",
    element: <App />,
    children: [
      { index: true, element: <HomePage /> },
      { path: "portfolio", element: <PortfolioPage /> },
      { path: "p/:project", element: <BoardPage /> },
      { path: "p/:project/state/:state", element: <StatusPage /> },
      { path: "p/:project/feature/:code", element: <FeaturePage /> },
      { path: "p/:project/milestones", element: <MilestonesPage /> },
      { path: "p/:project/milestone/:code", element: <MilestonePage /> },
      { path: "p/:project/schedule", element: <SchedulePage /> },
      { path: "p/:project/gantt", element: <GanttPage /> },
      { path: "p/:project/charter", element: <CharterPage /> },
      { path: "p/:project/review", element: <ReviewPage /> },
      { path: "p/:project/releases", element: <ReleasesPage /> },
      { path: "p/:project/workflow", element: <WorkflowPage /> },
      { path: "p/:project/docs", element: <DocsPage /> },
      { path: "p/:project/docs/folder/*", element: <DocsPage /> },
      { path: "p/:project/docs/file/*", element: <DocPage /> },
    ],
  },
]);

// No auth: the monitor is a read-only view of your local folder.
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>
);
