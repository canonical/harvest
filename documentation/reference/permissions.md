# Roles and permissions

## Server roles

| Action | Regular user | Admin |
|---|---|---|
| Own chats, artifacts, skills, API keys, code sessions | ✓ | ✓ |
| See public repositories | ✓ | ✓ |
| See private repositories | when their credentials can read them | all |
| Add repositories and versions, resynchronise | ✓ (repositories they can see) | ✓ |
| Delete versions and repositories | | ✓ |
| Manage users and roles (**Admin** page) | | ✓ |
| Global skills | use | use and manage |
| Deploy projects | as a member | as owner of every project |

Personal spaces (chats, personal artifacts and skills, code sessions) are visible to their owner only, administrators included.

The first account created on a server is an administrator.

## Project roles

Every request under `/projects/<id>` is classified:

| Request | Needs |
|---|---|
| `GET`, `HEAD` | Viewer |
| any other method, and opening a console | Editor |
| `DELETE /projects/<id>`, changing members | Owner |

Each role includes the ones below it. A project keeps at least one owner. Non-members get `404`, members with too low a role get `403`.

## Previews

| Audience | Who can open |
|---|---|
| `owner` | the session's owner |
| `repository` | signed-in users who can see the repository |
| `link` | anyone with the link |

## Code projects

Anyone who can see a repository can open it in Code mode. The project, index and image are shared; sessions are private to the user who started them.
