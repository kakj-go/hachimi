import { render } from "solid-js/web";
import { installFrontendLogging } from "@hachimi/contracts";
import "@hachimi/ui/styles";
import { WorkbenchApp } from "@hachimi/workbench";

installFrontendLogging();

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
render(() => <WorkbenchApp />, root);
