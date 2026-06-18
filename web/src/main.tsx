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
import DocsPage from "./pages/DocsPage";
import DocPage from "./pages/DocPage";

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
