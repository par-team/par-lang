# 패키지와 모듈

지금까지는 모듈 파일 하나의 내용에만 집중했다. 단일 파일도 좋은 시작이지만, 실질적으로 Par 프로그램은 **패키지**로 작성하며, 패키지는 여러 개의 **모듈**로 이루어져 있다.

프로그램의 계층 구조는 다음과 같다.

- **프로그램**이나 **라이브러리**가 하나의 **패키지**가 된다.
- **패키지**는 여러 개의 **모듈**로 이루어진다.
- **모듈**은 여러 개의 **타입 정의, 선언, 정의**로 이루어진다.

각각이 어떤 역할을 하는지 알아보자.

## 패키지

`Par.toml` 파일과 `src/` 디렉토리가 있는 프로젝트 디렉토리를 Par 패키지라고 한다.

예를 들어 보자.

```text
hello_par/
  Par.toml
  src/
    Main.par
```

`par new hello_par` 명령으로 생성되는 패키지가 바로 이것이다.

`Par.toml` 파일은 다음과 같이 시작한다.

```toml
[package]
name = "hello_par"
```

이 `name`이 패키지의 **권장 이름**으로, 문서 생성 등의 툴링에서 이 이름을 사용한다.

### 의존성

`[dependencies]` 섹션에 이 패키지가 의존하는 다른 패키지를 작성할 수 있다.

```toml
[package]
name = "postify"

[dependencies]
web = "github.com/author/par-web"
shared = "../shared"
```

각 의존성은 다음과 같은 형태로 작성한다.

```toml
alias = "reference"
```

좌변의 **alias**에는 `@web/...`과 같이 가져오기에서 사용할 이름을 작성한다.

우변의 **reference**는 둘 중 하나의 형태로 작성할 수 있다.

- **로컬 경로**. `.`, `..`, `~`, `$`으로 시작하는 것은 로컬 경로로 인식된다.
- **원격 의존성 소스**. `github.com/faiface/par-cancellable` 등 로컬 경로를 제외한 나머지는 모두 원격으로 취급한다.

로컬 경로는 다음과 같이 작성할 수 있다.

```toml
shared = "./shared"
common = "../common"
tools = "~/par/tools"
extras = "$PAR_PACKAGES/extras"
```

원격 의존성에는 아직 **버전 체계가 없으며**, 주어진 소스에 있는 최신의 내용을 그대로 가져온다.

### 원격 의존성의 관리

원격 의존성은 CLI를 통해 관리한다.

- `par add`를 하면 `Par.toml`을 읽고 누락된 원격 의존성을 `dependencies/`에 모두 다운로드한다.
- `par update`를 하면 CLI에서 관리하는 모든 원격 의존성을 재다운로드한다.
- `par add github.com/faiface/par-cancellable`를 하면 해당 의존성을 `Par.toml`에 권장 이름으로 추가하고 다운로드한다.

`dependencies/`디렉토리는 CLI에서 관리하므로, 직접 편집하면 안 된다.

간접 원격 의존성도 자동으로 다운로드된다. 여러 의존성에서 같은 원격 패키지를 사용할 경우, Par가 알아서 한 번만 다운로드한다.

로컬 의존성은 `dependencies/`로 복사되지 **않으며**, 디스크상의 위치 그대로 직접 참조한다.

### 내장 패키지

모든 패키지는 자동으로 다음 두 내장 패키지에 의존한다.

- `String`, `List`, `Map`, `Try` 등 핵심 타입과 자료구조를 담당하는 `@core`
- `Console`, `Os`, `Http` 등 단순한 입출력을 담당하는 `@basic`

이 두 패키지는 **암시적 의존성**에 해당하지만, **암시적 가져오기**는 되지 않는다. 해당 모듈은 가져오기로 사용할 수 있지만, 가져오지 않는 이상 소스 파일의 범위에 자동으로 포함되지는 않는다.

### `par doc`을 사용한 패키지 탐색

특정한 패키지의 내용을 확인해 보고자 한다면 `par doc`으로 모두 해결할 수 있다.

- **패키지 바깥**에서 `par doc`을 실행하면 **내장 패키지**를 확인할 수 있다.
- **패키지 안**에서 `par doc`을 실행하면 **현재 패키지와 의존하는 패키지**를 확인할 수 있다.
- **원격 패키지**의 경우, 의존성으로 직접 추가하지 않아도 `par doc --remote github.com/faiface/par-cancellable` 명령으로 패키지의 내용을 확인해볼 수 있다.

## 모듈

모듈은 `src/` 아래에 작성하며, 디렉토리 구조는 원하는 대로 구성하면 된다.

예를 들어 보자.

```text
src/
  Main.par
  data/
    Post.par
  handlers/
    api/
      Posts.par
```

이 패키지의 구조는 다음과 같다.

- `src/Main.par` 파일에서는 `Main` 모듈을 정의한다.
- `src/data/Post.par` 파일에서는 `data/Post` 경로의 `Post` 모듈을 정의한다.
- `src/handlers/api/Posts.par` 파일에서는 `handlers/api/Posts` 경로의 `Posts` 모듈을 정의한다.

모듈의 이름이 두 부분으로 나뉘는 것을 알 수 있다.

- 모듈에는 `Post`와 같이 **이름**이 있다.
- 모듈에는 역시 `data/Post`와 같이 **경로**가 있다.

모듈의 **이름**은 파일 내부에서 사용한다.

```par
module Post
```

모듈을 다른 곳에서 가져올 때는 **경로**를 사용한다. 파일명과 모듈 선언은 반드시 일치해야 하지만, **대소문자를 따지지 않는다**.

즉,

- `Post.par`에서는 `module Post`를 선언해야 한다.
- `post.par`에서도 `module Post`를 선언할 수 있다.
- `handlers/api/Posts.par`에서는 `module Posts`를 선언해야 한다.

디렉토리는 모듈의 **경로**를 이루지만, 선언되는 모듈 이름과는 무관하다.

## 모듈 가져오기

모듈에서는 다른 모듈을 명시적으로 가져올 수 있다.

**같은 패키지**에서 모듈을 가져오려면 `src/` 아래의 절대 경로를 사용하면 된다.

```par
import data/Post
import handlers/api/Posts
```

상대 경로로 가져오기는 지원하지 **않는다**.

가져오기 경로에서는 항상 정방향 슬래시를 사용한다.

```par
import util/DateTimeUtils
```

**의존성**에서 모듈을 가져오려면 경로 앞에 `@alias`를 붙이면 된다.

```par
import @web/http/Server
import @shared/FancyModule
```

### 다른 이름으로 가져오기

두 모듈의 이름이 충돌할 경우, 다른 이름을 지정해서 가져올 수 있다.

```par
import @dep1/blah/Data as Data1
import @dep2/bleh/Data as Data2
```

### 묶어서 가져오기

여러 모듈을 하나의 구문으로 묶을 수 있다.

```par
import {
  @basic/Console
  @core/List
  data/Post
}
```

묶어서 가져오기는 다중 `import`문의 단순 문법 설탕이다.

## 가져온 모듈의 이름 사용하기

모듈을 가져왔다면 그 모듈에서 내보낸 요소를 모듈 이름을 통해 사용할 수 있다.

```par
import {
  @core/List
  @core/String
  data/Post
}

dec RenderPosts : String
def RenderPosts = `#{Post.FetchAllFromDB(!)->List.Length} posts`
```

가져온 이름은 일반적으로 다음과 같이 사용한다.

```par
Module.Name
```

## 주요 타입과 주요 선언

모듈에서는 **타입**과 **선언** 하나씩을 모듈 자신과 같은 이름으로 내보낼 수도 있다.

이렇게 내보낸 특별한 타입과 선언은 모듈 이름만으로 직접 사용할 수 있다.

예를 들어 보자.

```par
module Post

export {
  type Post = box choice {
    .title => String,
    .content => String,
  }

  dec Post : [String, String] Post
  dec FetchAllFromDB : [!] List<Post>
}
```

다른 모듈에서 이 모듈을 가져오면,

```par
import data/Post
```

다음 요소를 사용할 수 있다.

- `Post`를 그대로 **타입**으로
- `Post`를 그대로 **선언**으로
- 모듈에서 내보낸 다른 선언 `Post.FetchAllFromDB`

다른 이름을 사용할 때도 문제 없이 사용할 수 있다.

```par
import @core/List as L
```

이렇게 가져왔을 때는,

- `L<a>`가 모듈의 주요 타입이 된다.
- `L.Map`, `L.Filter` 등 내보낸 다른 선언도 사용할 수 있다.

Par의 타입과 값은 문법상의 위치로 항상 구분할 수 있기 때문에 주요 타입 `Post`와 주요 선언 `Post`가 같이 있어도 문제가 되지 않는다.

## 접근 제어와 내보내기

접근 제어에 관해 두 가지의 연관된 질문을 할 수 있다.

1. 패키지 밖에서 **모듈**에 접근할 수 있는가?
2. 모듈 밖에서 **타입**이나 **선언**에 접근할 수 있는가?

### 모듈 내보내기

모듈은 `export module`과 무관하게 **같은 패키지 안**에서 항상 접근할 수 있다.

특정 모듈을 **의존하는 패키지**에게 공개하려면 다음과 같이 표시하면 된다.

```par
export module List
```

### 요소 내보내기

타입과 선언은 **기본적으로 모듈 안에서만 접근할 수 있다**.

특정 요소를 모듈 밖에 공개하려면 `export`를 사용하면 된다.

```par
export type Iterator<a> = ...
export dec Map : ...
```

묶어서 내보내기도 가능하다.

```par
export {
  type Iterator<a> = ...
  dec Map : ...
  dec Filter : ...
}
```

`export def`는 **없는 문법이다**.

정의는 항상 구현하는 측에 작성한다. 특정한 값을 공개하려면 해당하는 **선언**을 내보내면 된다.

### 접근 제어의 3단계

위의 두 계층을 조합하여 모듈 요소에 사실상 세 단계의 접근 제어가 가능하다.

1. `export module`의 내보낸 요소  
   해당 요소는 의존하는 패키지에 공개된다.
2. 내보내지 않은 모듈의 내보낸 요소  
   해당 요소는 같은 패키지에 공개되지만, 패키지 바깥에서는 접근할 수 없다.
3. 모든 모듈의 내보내지 않은 요소  
   해당 요소는 같은 모듈 안에서만 접근할 수 있다.

### 타입 수준의 접근 제어 검사

Par에서는 공개된 API에 비공개 타입을 사용하지는 않는지도 확인한다.

예를 들어, 특정한 선언이 패키지 내부나 패키지 바깥에 공개되어 있을 경우에는 그 타입에서 덜 공개된 타입을 사용해서는 안 된다. 즉,

- 모듈에만 공개된 타입을 패키지에 공개된 요소에서 사용해서는 안 된다.
- 패키지에만 공개된 타입을 패키지 밖에 공개된 요소에서 사용해서는 안 된다.

이 방법으로 덜 공개된 헬퍼 타입이 공개 API에 노출되는 것을 막을 수 있다.

## 순환 참조

이전 장에서 타입과 정의가 서로 순환 참조를 이루어서는 안 된다는 것을 살펴보았듯, 패키지 역시 순환 의존성을 이루어서는 안 된다.

단, 모듈의 경우 같은 패키지의 모듈이 서로 순환 가져오기를 이루는 것은 가능하다.

어차피 순환 참조 정의가 불가능한데 굳이 모듈 순환 가져오기가 필요한 이유가 있을까? 이는 여러 모듈에서 서로의 타입을 사용해야 할 때가 있으며, 실제로 순환 참조를 이루지만 않는다면 한 모듈의 정의에서 다른 모듈의 정의를 호출하는 것도 문제가 없기 때문이다.

예를 들어, 내장 `@core/List` 모듈에서는 `List.Length`가 `Nat`을 반환하기 때문에 `@core/Nat`을 가져온다. 한편, `@core/Nat`에서도 `Nat.Range`가 `List<Nat>`을 반환하기 때문에 `@core/List`를 가져온다.

모듈끼리 서로를 가져오고 있지만, 모듈 안의 정의 자체에서는 순환 참조가 생기지 않았다.

## 다중 파일 모듈

파일 하나가 너무 커질 경우, 같은 디렉토리의 여러 파일에 걸쳐 모듈을 나눌 수 있다.

```text
src/
  Parser.par
  Parser.lexing.par
  Parser.errors.par
```

위의 세 파일은 모두 같은 모듈에 속한다.

```par
module Parser
```

모듈 나누기의 규칙은 다음과 같다.

- 같은 디렉토리에서 `Parser.par`와 `Parser.*.par` 형태의 모든 파일이 하나의 모듈을 이룬다.
- 모든 파일에서 `module Parser`를 선언해야 한다.
- 모든 파일에서 동일한 최상위 모듈 이름공간을 공유한다.
- 각 파일마다 **가져오기가 별도로 이루어지며**, 해당 파일에만 적용된다.
- 모든 파일에서 `export module` 표시 여부가 동일해야 한다.

## 정의의 실행

`par run`으로는 다른 언어에서 `null`이나 빈 튜플에 대응하는 단위 타입 `!`의 정의만 실행할 수 있다.

이외의 제네릭이 아닌 정의는 플레이그라운드에서 실행할 수 있으며, 해당 정의의 타입에 따라 생성되는 자동 UI에서 상호작용이 가능하다.

모듈에 경로가 있는 것을 배웠으니, `par run`의 대상 문법 역시 같은 방법으로 이해할 수 있다.

- 실행 대상은 다음 중 하나의 형태를 가진다.
  - `path/to/Module`
  - `path/to/Module.Def`
- 슬래시로 구분된 부분은 항상 **모듈 경로**가 된다.
- 실행 대상이 모듈 경로에서 바로 끝날 경우, `par run`에서는 그 모듈의 `Main` 정의를 찾아 실행한다.
- 실행 대상이 `.Def`로 끝날 경우, Par에서는 모듈에서 해당 정의를 찾아 실행한다.

즉,

- `par run`을 하면 `Main.Main`이 실행된다.
- `par run Main`을 해도 `Main.Main`이 실행된다.
- `par run Main.Other`를 하면 `Main` 모듈의 `Other` 정의가 실행된다.
- `par run handlers/api/Posts`를 하면 `handlers/api/Posts.Main` 정의가 실행된다.
- `par run handlers/api/Posts.Program`를 하면 `handlers/api/Posts` 모듈의 `Program` 정의가 실행된다.

`par test`나 `par check` 등의 다른 명령은 위와 같은 실행 대상을 입력받지 않고, 패키지 전체를 대상으로 하여 실행된다. `par run`을 포함하여 패키지를 대상으로 하는 모든 명령은 `--package` 플래그를 사용하여 원하는 패키지의 경로를 지정할 수 있다.

이제 패키지/모듈 시스템을 모두 다루었으니 언어 자체를 계속 살펴 보자.
