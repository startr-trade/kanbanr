import React from "react";
import ReactDOM from "react-dom/client";
import { createBrowserRouter, createHashRouter, Navigate, RouterProvider, type RouteObject } from "react-router-dom";
import "./styles.css";
import App from "./App";
import HomePage from "./pages/HomePage";
import PortfolioPage from "./pages/PortfolioPage";
import BoardPage from "./pages/BoardPage";
import StatusPage from "./pages/StatusPage";
import FeaturePage from "./pages/FeaturePage";
import MilestonesPage from "./pages/MilestonesPage";
import MilestonePage from "./pages/MilestonePage";
import PlanPage from "./pages/PlanPage";
import CharterPage from "./pages/CharterPage";
import CatalogPage from "./pages/CatalogPage";
import ReviewPage from "./pages/ReviewPage";
import WorkflowPage from "./pages/WorkflowPage";
import BaselinePage from "./pages/BaselinePage";
import ItemsPage from "./pages/ItemsPage";
import DocsPage from "./pages/DocsPage";
import DocPage from "./pages/DocPage";
import ReleasesPage from "./pages/ReleasesPage";
import UiGalleryPage from "./pages/UiGalleryPage";
import { loadConfig, routing } from "./runtimeConfig";

const routes: RouteObject[] = [
  {
    path: "/",
    element: <App />,
    children: [
      { index: true, element: <HomePage /> },
      { path: "portfolio", element: <PortfolioPage /> },
      // The UI kit's gallery (FTR-046): in every build, linked from nowhere.
      { path: "ui", element: <UiGalleryPage /> },
      { path: "p/:project", element: <BoardPage /> },
      { path: "p/:project/state/:state", element: <StatusPage /> },
      { path: "p/:project/feature/:code", element: <FeaturePage /> },
      { path: "p/:project/milestones", element: <MilestonesPage /> },
      { path: "p/:project/milestone/:code", element: <MilestonePage /> },
      // One Plan page in place of the Schedule and the Gantt (FTR-043); their old URLs still land on it.
      { path: "p/:project/plan", element: <PlanPage /> },
      { path: "p/:project/schedule", element: <Navigate to="../plan?tab=planned" relative="path" replace /> },
      { path: "p/:project/gantt", element: <Navigate to="../plan" relative="path" replace /> },
      { path: "p/:project/charter", element: <CharterPage /> },
      { path: "p/:project/catalog", element: <CatalogPage /> },
      { path: "p/:project/review", element: <ReviewPage /> },
      { path: "p/:project/items", element: <ItemsPage /> },
      { path: "p/:project/releases", element: <ReleasesPage /> },
      { path: "p/:project/workflow", element: <WorkflowPage /> },
      { path: "p/:project/baseline", element: <BaselinePage /> },
      { path: "p/:project/docs", element: <DocsPage /> },
      { path: "p/:project/docs/folder/*", element: <DocsPage /> },
      { path: "p/:project/docs/file/*", element: <DocPage /> },
    ],
  },
];

// The runtime configuration comes first (FEAT-178): it says where the API is and which router a
// static host can support. A missing config.json means the embedded defaults, so this never blocks.
// No auth: the monitor is a read-only view of your local folder.
loadConfig().then(() => {
  const router = routing() === "hash" ? createHashRouter(routes) : createBrowserRouter(routes);
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <RouterProvider router={router} />
    </React.StrictMode>
  );
});
