import { PetApp } from "@hachimi/pet";
import { installFrontendLogging } from "@hachimi/contracts";
import "@hachimi/ui/styles";
import { render } from "solid-js/web";

installFrontendLogging();

const root = document.getElementById("root");
if (!root) throw new Error("pet root element is missing");
render(() => <PetApp />, root);
