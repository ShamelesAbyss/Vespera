# Vespera Nexus Studio on Microsoft Store

Vespera Nexus Studio is the reserved Microsoft Store name for Vespera. The owner supplied Store product ID `9PNV5XMS74FW` and reported that the product is in draft. The repository and application branding use Vespera.

## Confirm the package identity

Open the [Vespera Nexus Studio draft](https://partner.microsoft.com/en-us/dashboard/products/9PNV5XMS74FW/overview) in the owner's signed-in Partner Center session. Under **Product management → Product identity**, copy these fields exactly. [Microsoft identity reference](https://learn.microsoft.com/windows/apps/publish/view-app-identity-details)

| Partner Center field | Local JSON field |
| --- | --- |
| Store ID | `storeProductId` |
| Package/Identity/Name | `name` |
| Package/Identity/Publisher | `publisher` |
| Package/Properties/PublisherDisplayName | `publisherDisplayName` |
| Package Family Name | `packageFamilyName` |

Also confirm that **Packages** accepts a Windows Desktop x64 MSIX for this draft. No authenticated Partner Center session or provisioned submission API was available during development. Keep the identity JSON in an operator-controlled location, such as `target/store-identity.json`. It contains identity metadata only. Microsoft account credentials and tokens belong in the owner's authorized authentication flow. The build checks the supplied family against the Windows-derived family for the exact name and publisher before staging any files. [Windows package identity](https://learn.microsoft.com/windows/apps/desktop/modernize/package-identity-overview)

## Build an unsigned MSIX

Use PowerShell 7 on a Windows x64 host with the pinned Rust toolchain, MSVC build tools and Windows SDK MakeAppx. Run from the repository root.

```powershell
./scripts/package-windows-store.ps1 -Development -Version 1.0.0.0 -Output target/store-development
./scripts/package-windows-store.ps1 -IdentityFile target/store-identity.json -Version 1.0.0.0 -Output target/store-candidate
```

Each output directory must be new. Development uses a separate package family and a visible **Development** label in the Start Menu. The Store path requires a clean checkout and the actual identity file. Both build the optimized locked Rust/Bevy desktop, stage an explicit payload, validate and unpack it with MakeAppx, compare staged and unpacked hashes, and emit the unsigned MSIX, a 300×300 Store listing tile and build evidence. The Store package version is independent of the Rust application version. Start at `1.0.0.0` only if Partner Center confirms it is available. Increase the version for upgrades and leave the fourth component at zero. [Microsoft package version rules](https://learn.microsoft.com/windows/apps/publish/publish-your-app/app-package-requirements?pivots=store-installer-msix)

| Package decision | Behavior |
| --- | --- |
| Native entry point | Windows launches the optimized `vespera-view.exe` directly. Vespera's MSI and portable launcher keep their existing entry points. |
| Runtime | Full-trust packaged desktop app at medium integrity, with `runFullTrust` and the Microsoft Visual C++ Desktop framework dependency. The MSIX does not bundle a driver or model server. |
| Storage | WinRT `ApplicationData.LocalFolder` supplies the package's per-user LocalState. A package upgrade keeps the family and data. Copy backed-up data explicitly when migrating between the classic and Store distributions. Export important data before removing the MSIX. |
| Provider access | The desktop uses its existing loopback provider declarations and credential references. The provider daemon is installed and managed independently. No firewall rule, AppContainer loopback exemption or external executable installation is part of the MSIX. |
| Child processes | The package starts the viewer directly. Store builds use Microsoft Store updates and suppress Vespera's standalone Setup handoff. |
| Operating system | The manifest targets Windows Desktop x64 with a Windows 10 build 19041 API floor. Test the current supported Windows editions and drivers separately before declaring support. |

The existing Windows MSI, Setup EXE and portable ZIP have their own release pipeline. Microsoft Store package signing occurs through the Store workflow and applies to the Store package. [MSIX container behavior](https://learn.microsoft.com/windows/msix/msix-containerization-overview) · [Desktop CRT framework](https://learn.microsoft.com/troubleshoot/developer/visualstudio/cpp/libraries/c-runtime-packages-desktop-bridge) · [MSIX signing](https://learn.microsoft.com/windows/msix/package/signing-package-overview)

## Development installation

Use an isolated Windows test machine or disposable test user with the required x64 VCLibs framework. Build two versions under the development identity, for example `1.0.0.0` and `1.0.1.0`. Sign copies with the same non-exportable development code-signing certificate whose subject matches the manifest. Trust its public certificate in Local Computer **Trusted People** for the test. Remove the isolated test package and temporary certificate when finished. Keep the unsigned originals for verification. [Microsoft development-signing procedure](https://learn.microsoft.com/windows/msix/package/sign-msix-package-guide)

Install using `Add-AppxPackage`, open **Vespera Nexus Studio (Development)** from Start, and check the branded Home frame. Exercise provider discovery, inference, agent and workflow execution, retained memory, simulation, close and restart. Upgrade using the newer MSIX and confirm the same package family, retained revision, settings and owned records. Run the Windows App Certification Kit on the candidate and retain its report separately from functional evidence.

The Windows development rehearsal built and unpacked both versions with SDK 10.0.26100.0. SignTool signed isolated copies with a temporary non-exportable certificate. Windows installed both versions and started the branded native viewer from the package identity. A fresh profile committed a Living World tick, discovered an Ollama-protocol loopback fixture, registered two agents, and completed a two-agent workflow. It recalled the first agent's owned response in a later provider request. Swarm showed two agents and three observed links from the committed journal. The upgrade reopened world revision 1 and preserved the world, task, workflow and memory database bytes. The development package and certificate were removed after each run.

The clean-source unsigned packages were built at `399c2d015eae6aac506dcd02624a6caba258085c`. Both had 14 payload files verified through MakeAppx and the upgrade payload matched the installed test payload. The internal review ZIP contains the development package, listing tile, screenshot, CycloneDX 1.6 SBOM and collected notices. OSV returned zero matches for the 429 exact Cargo versions queried. Live packaged model inference, controlled-interruption recovery on MSIX, Windows App Certification Kit results and the real Store identity remain separate qualification gates.

| Internal artifact | SHA-256 |
| --- | --- |
| Development MSIX `1.0.0.0` | `e023014df87ae78703928cac8cc13acef07955e389c2768c6c431165d93f183d` |
| Development MSIX `1.0.1.0` | `b66d5bfabfb3105125ccb11956e26af0d4023483264f72dfa3ff2356c46b8d0f` |
| Development review ZIP | `0702bc40f95a4eac46c29cd61009ffbba863798de6a58ee96a0c4e321c55d5b1` |

## Draft Store listing

**Description**

Vespera Nexus Studio is a local-first workspace for AI models, agents, and simulated worlds. Connect a compatible local model server, assign models and roles to agents, and run workflows whose inputs, results, and ownership can be inspected. Explore a persistent 3D settlement and compare repeatable simulation outcomes using saved trials.

**Feature list**

- Discover models on local Ollama and OpenAI-compatible endpoints.
- Configure agent roles, model bindings, and explicit permissions.
- Author workflows with dependencies and inspect retained results.
- Keep permission-scoped agent memory and local message history.
- Inspect observed activity in Swarm and view host resources in Machine.
- Run Living World, replay saved revisions, and compare starter-world policies.

**Requirements and accessibility**

Windows x64 and a working graphics driver for Bevy/wgpu rendering. Model inference requires a separately running compatible provider and enough memory for the chosen model. Windows GPU and driver combinations beyond the recorded validation scope require qualification. Home and Atlas provide keyboard navigation and a reduced-motion setting. Screen-reader coverage and accessibility certification remain pending.

**Visual assets**

Use the generated listing tile and native package captures. The 1464×841 Living World image below comes from the installed development MSIX after committing the first simulation tick. Caption it “Living World shows a committed simulation revision and interactive resident and resource inspection.” Capture remaining views at 1366×768 or larger in PNG. Label fixture activity where shown and exclude account information, filesystem paths, private prompts and credentials. [Microsoft screenshot requirements](https://learn.microsoft.com/windows/apps/publish/publish-your-app/msix/screenshots-and-images)

![Living World running in the installed Vespera Nexus Studio development package](../assets/screenshots/store-living-world.png)

**Privacy and permissions review**

Prompts, responses, settings, task history, messages, memory and world data are stored locally when the relevant feature is used. An explicitly configured provider receives the task inputs it needs, which can include selected memory or preceding workflow outputs. The desktop reads host resource information for Machine and can read user-provided clipboard text during editing. The application uses full trust to run the native renderer, access its local SQLite files, and reach operator-configured local services. Review third-party notices against the MSIX payload and resolve outstanding rights and attribution before distribution. [Privacy and support requirements](https://learn.microsoft.com/windows/apps/publish/publish-your-app/msix/support-info) · [Vespera release review](RELEASE.md)

**Proposed support URL for owner review** [Vespera issue tracker](https://github.com/ShamelesAbyss/Vespera/issues)

**Draft privacy text for owner review**

> Vespera Nexus Studio stores the work you create and the settings you choose in your Windows user profile. When you run an inference task, Vespera sends the prompt and any context you authorize to your selected model provider. Context can include saved memory or earlier workflow results. Your provider controls how it processes and retains that input. Vespera reads local hardware information for the Machine view and reads clipboard text when you paste it into an editor. Windows manages the installed package and its updates. If you report a problem through the public issue tracker, include only the details you want to make public. Keep backups of your work before removing the application.

The privacy text, disclosure answers and support route need owner review before submission.

## Prepare the submission

Use **Packages** in the authenticated draft to validate the manifest and upload a candidate after review. Complete Store listings, Properties, Age ratings, Privacy and support, availability and certification notes. An authorized Entra application associated with Partner Center can use the MSIX submission API for read and draft operations. Its documented read endpoint is `GET https://manage.devcenter.microsoft.com/v1.0/my/applications/{applicationId}`. Use the app ID returned by the authorized account listing. Provisioning access and credentials requires the owner's authorized workflow. Keep upload links and credentials out of logs and source. Final public submission, legal agreements, new persistent access and costs require owner authorization. [Read application data](https://learn.microsoft.com/windows/uwp/monetize/get-app-data) · [Create an MSIX submission](https://learn.microsoft.com/windows/apps/publish/publish-your-app/msix/create-app-submission)
