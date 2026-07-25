import { render } from "solid-js/web";
import { installFrontendLogging } from "@hachimi/contracts";
import { WorkbenchApp } from "@hachimi/workbench";
import "@hachimi/ui/styles";

installFrontendLogging();

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
render(() => <WorkbenchApp />, root);
