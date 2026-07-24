# 오류 처리

파일이 없거나, 네트워크 연결이 끊기거나, 사용자가 잘못된 입력을 하는 등, 실 세계와 상호작용하는 프로그램이라면 오류를 깔끔하게 처리해야 한다. 대부분의 오류는 프로그램이 자신의 영역 밖인 외부 시스템과 접촉하는 입출력 경계에서 발생한다.

Par는 선형 타입 시스템에 기반한 구조화된 오류 처리 방법을 사용한다. Par의 기반에서는 명시적인 시도(Try) 타입을 사용하지만, 그 위에 경량의 문법 설탕을 추가하여 기저의 의미론을 투명하게 공개하면서도 오류를 자연스럽게 처리할 수 있도록 하였다.

## Par에 고유한 오류 처리가 필요한 이유

선형 타입 시스템과 동시적 평가가 맞물리면 Par만의 특수한 오류 처리 문제가 발생하게 된다. 기존의 접근은 여러 이유로 Par와 맞지 않는다.

**예외**는 호출 스택을 거슬러 전파되면서 자동으로 여러 겹의 함수 호출을 되돌린다. 하지만 Par의 동시성 실행 모델에는 호출 스택이라는 개념이 없고 채널로 통신하는 프로세스만이 있을 뿐이다! 모든 오류는 채널을 통해 명시적으로 전달되어야 하므로 오류 처리에 `Try` 타입과 같은 장치가 필수이다.

**Rust의 `?` 연산자**는 오류를 전파할 때 소유자가 있는 값을 버린다. Par의 선형 타입 시스템에서는 모든 값이 각자의 타입과 맥락에 맞게 사용되어야 하므로 이와 같은 암시적 정리 역시 도입하기 어렵다.

Par의 오류 처리는 값 정리가 명시적으로 이루어지면서도 사용하기 편리해야 한다. 아래에서 소개하는 `try`/`catch`/`throw` 문법 설탕은 예외 처리에서 자주 쓰이는 키워드를 빌리면서도 매우 다르게 동작하여 두 가지 문제를 모두 해결한다. 기존의 예외와 달리 Par의 오류 처리는 숨은 제어 흐름이나 스택 되돌림 없이 `Try` 타입에 대한 완전히 국소적인 문법 설탕으로 구현되어 있다.

## 파일 다루기: 문법 설탕 없는 오류 처리

Par의 내장 `@basic/Os` 모듈에서 제공하는 파일시스템 연산을 사용하는 예제를 확인해 보자. `Os.Path` 타입에서는 파일을 만들고 디렉토리를 읽는 등 파일시스템 작업을 수행하는 메서드를 제공한다. 대부분의 연산은 실패할 수 있으므로 `Try` 값을 반환한다.

문법 설탕을 사용하지 않고 오류 처리를 해 보자. 여기에서는 로그 파일을 생성하고 몇 줄의 로그를 작성하는 프로그램을 작성한다.

```par
module Main

import {
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  let path = Os.Path("logs.txt")
  Os.CreateOrAppendToFile(path).case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok writer => {}
  }
  // ...
```

이 패턴에 대해 몇 가지 특기할 점이 있다.

`chan exit`에서는 [후속문 타입](../processes/duality.md)인 `?` 타입을 가지는 채널 `exit`를 생성한다. `exit!` 문법은 이 후속문에 적용되는 *탈출* 명령으로, 프로세스를 종료한다.

`.case`문이 종료되면 그 바깥 범위에 `writer` 변수가 노출된다. Par에서는 프로세스 범위의 변수가 이 방식으로 동작한다. 즉, `.case` 분지에서 대입된 변수는 분기 분석이 종료되어도 사라지지 않는다.

```par
  writer.write("[INFO] First new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }
```

[프로세스 문법](../process_syntax.md)에서 `.ok =>`를 하면 명령의 주어(`writer`)가 `.ok` 분지의 페이로드로 갱신된다. `.write` 메서드는 성공 시 `Os.Writer`를 반환하므로 `writer`의 쓰임에는 변함이 없다.

```par
  writer.write("[INFO] Second new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }
```

마지막으로 파일을 닫는다.

```par
  writer.close.case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok! => {}
  }
  exit!
}
```

`.ok!` 패턴에 주의해야 한다. 파일을 닫고 나면 `writer`는 단위 값 `!`이 된다.

완성된 프로그램은 다음과 같다.

```par
module Main

import {
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  let path = Os.Path("logs.txt")
  Os.CreateOrAppendToFile(path).case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok writer => {}
  }
  
  writer.write("[INFO] First new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }
  writer.write("[INFO] Second new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }

  writer.close.case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok! => {}
  }

  console.close
  exit!
}
```

지나치게 번잡하다! 실패할 수도 있는 연산을 실행할 때마다 똑같은 오류 처리 코드가 반복되어 있다. Par의 오류 처리 문법 설탕을 사용하면 코드가 어떻게 개선되는지 확인해 보자.

## `try`/`catch`로 재작성한 프로그램

Par의 오류 처리 문법을 사용해 위와 동일한 기능을 구현하면 다음과 같다.

```par
module Main

import {
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  catch e => {
    console.print(e)
    console.close
    exit!
  }

  let path = Os.Path("logs.txt")
  let try writer = Os.CreateOrAppendToFile(path)
  
  writer.write("[INFO] First new log\n").try
  writer.write("[INFO] Second new log\n").try

  writer.close.try
  console.close
  exit!
}
```

훨씬 짧고 가독성이 좋다! 오류 처리 로직을 한 번 선언한 뒤 그 뒤의 모든 연산에 적용하고 있다.

## 프로세스 문법에서 `try`/`catch`/`throw`의 원리

Par의 오류 처리 문법 설탕은 명시적 `Try` 처리로 변환되는 작고 국소적인 키워드로 이루어져 있다. 오류 처리의 원리를 이해해 보자.

### `catch`문

`try`나 `throw`를 하기 전에는 같은 프로세스에서 반드시 `catch` 블록을 정의해 두어야 한다. '같은 프로세스'라는 조건이 중요하다. `try`나 `throw` 명령은 반드시 중첩된 프로세스나 식이 아니라 동일한 순차 프로세스에서 작성해야 한다.

```par
catch <pattern> => {
  <process>
}
```

`<pattern>`에는 `let`문이나 함수 매개변수에서 사용하는 패턴을 그대로 사용하면 된다. 웬만하면 간단히 변수명을 작성해도 되지만, 필요할 때는 더 복잡한 패턴도 사용할 수 있다.

예를 들어, 단위 타입을 오류로 사용할 때는 이렇게 작성한다.

```par
catch ! => { ... }
```

타입 표기 역시 삽입할 수 있다.

```par
catch e: Os.Error => { ... }
```

`catch` 블록 안의 `<process>`는 반드시 프로세스 종료 명령을 사용해 종료하여야 한다.

- *탈출*: `continuation!`
- *연결*: `left <> right`
- `.loop`를 사용해 `catch` 블록 바깥의 `.begin`으로 반환. 연산 재시도에 적합하다.
- `throw`를 사용해 다른 `catch` 블록으로 점프

### `throw` 명령

`throw`는 오류 값을 가지고 `catch` 블록으로 직접 점프한다.

```par
catch e => {
  console.print(e)
  console.close
  exit!
}

throw "Total meltdown"
```

이는 해당하는 `catch` 블록을 직접 실행하는 것과 동일하다.

```par
console.print("Total meltdown")
console.close
exit!
```

`throw`는 로직 내에 사용자가 원하는 오류 조건을 구현할 때 사용할 수 있다.

## `try` 패턴과 명령

오류 처리의 핵심은 `Try` 값에 기반해 조건부로 오류를 처리하는 `try`에 있다.

```par
type Try<e, a> = either {
  .err e,
  .ok a,
}
```

`try`에는 *패턴*과 *명령*의 두 종류가 있다.

### 명령 형태의 `.try`

`.try` 명령으로 번잡한 `Try` 분기 분석을 깔끔한 선형 코드로 변환할 수 있다. 원래의 번잡한 코드를 다시 읽어 보자.

```par
writer.write("[INFO] First new log\n").case {
  .err e => {
    console.print(e)
    console.close
    exit!
  }
  .ok => {}
}
```

`.try`를 사용하면 다음과 같이 재작성할 수 있다.

```par
writer.write("[INFO] First new log\n").try
```

`.try` 명령은 `Try`를 반환하는 어떤 명령이나 식에든 적용할 수 있다.

```par
variable.try
```

위의 코드는 다음과 같이 변환된다.

```par
variable.case {
  .err e => {
    throw e
  }
  .ok => {}
}
```

더욱 복잡한 명령 연쇄도 가능하다. 데이터를 폴링하되 오류가 발생할 수 있음을 나타내는 다음 타입을 생각해 보자.

```par
type Poll<e, a> = iterative choice {
  .close => Try<e, !>,
  .next => Try<e, (a) self>,
}
```

값을 폴링하고 오류를 처리하는 것 역시 자연스럽게 가능하다.

```par
// source : Poll<Os.Error, String>
source.next.try[value]
```

이 명령이 실행되면 `source`는 `Poll<Os.Error, String>` 타입을 유지하고 `value`에는 폴링에 성공한 `String`이 대입된다.

<!-- moved `default` to the end of this chapter -->

### 동시적 평가의 제한

얼핏 보면 이 코드에 문제가 없다고 생각할 수 있다.

```par
let writer = Os.CreateOrAppendToFile(path).try
```

이 코드는 실제로는 타입 오류를 일으킨다. 이 오류가 발생하는 원인은 Par의 근본적인 실행 모델과 밀접하게 연관되어 있다.

Par는 식을 평가하는 동안 그 식의 사용자 역시 동시적으로 실행된다. 다음과 같은 코드를 작성하면...

```par
let writer = Os.CreateOrAppendToFile(path).try
```

식 `Os.CreateOrAppendToFile(path)`은 `let`문을 실행하는 프로세스와 동시적으로 실행된다. 해당 식이 `.try`에서 실패하는 시점에 메인 프로세스에서는 이미 다른 명령을 실행하고 있을 수도 있으며, 이미 실행된 내용을 안전하게 '되돌리는' 방법은 없다.

`try`와 `throw`를 중첩된 식이나 프로세스가 아니라 대응하는 `catch`와 정확히 같은 프로세스에서만 사용할 수 있는 이유가 바로 이것이다.

### 패턴 형태의 `try`

이 문제를 해결하려면 패턴 자체에서 `try`를 사용하면 된다.

```par
let try writer = Os.CreateOrAppendToFile(path)
```

이렇게 작성해야 올바른 프로세스에서 오류 처리의 책임을 맡는다. 변환된 코드는 다음과 같다.

```par
let writer = Os.CreateOrAppendToFile(path)
writer.case {
  .err e => {
    throw e
  }
  .ok => {}
}
```

`try` 자체가 패턴 문법이므로 다른 패턴 안에 중첩시키는 것도 가능하다.

```par
let (try leftReader, try rightReader)! = (
  Os.OpenFile(leftPath),
  Os.OpenFile(rightPath),
)!
```

수신 명령에서 역시 사용할 수 있다. `Console` 타입이 좋은 예시가 된다.

```par
type Console = iterative choice {
  .close => !,
  .print(String) => self,
  .prompt(String) => (Try<!, String>) self,
}
```

`.prompt` 메서드는 `Try`를 반환하고, 이후에도 콘솔 핸들이 유지되어 다른 연산을 할 수 있다.

```par
let console = Console.Open

catch ! => {
  console.print("Failed to read input.")
  console.close
  exit!
}

console.prompt("What's your name?")[try name]
console.prompt("What's your address?")[try address]
```

## 식 문법에서의 오류 처리

Par는 식에서도 `try`/`catch`를 직접 지원하며, 식의 맥락에 맞게 문법 역시 다소 수정되어 있다.

```par
catch <pattern> => <err result> in <expression using try/throw>
```

동시적 평가의 제한이 동일하게 적용되며, 추가로 `try`/`throw`는 결과값이 조금이라도 생성되기 전에만 사용할 수 있다는 제한이 있다.

아래의 코드에서 `result.try`는 별도의 동시적 프로세스로 실행되는 중첩된 식에 등장하므로 컴파일이 되지 않는다.

```par
// result : Try<String, Int>
catch e => .err e in
.ok {result.try + 1}
```

중첩된 식 문제를 피하기 위해 다음과 같이 코드를 수정해도 컴파일이 되지 않는다. `try`가 실행되기 전에 바깥의 `.ok`가 먼저 생성되기 때문이다.

```par
catch e => .err e in
.ok let try value = result in
value + 1
```

다음과 같이 수정해야 컴파일이 가능하다.

```par
catch e => .err e in
let try value = result in
.ok {value + 1}
```

위와 같이 코드를 작성해야 결과값을 생성하기 전에 오류 처리가 완료된다.

### 자주 쓰이는 식 패턴

식 형태의 `catch`는 여러 가지 패턴에 자주 쓰인다.

#### 오류 매핑 (맥락 추가하기)

```par
catch e => .err String.Builder.add("Failed to process file: ").add(e).build in
let try content = file.readAll in 
.ok ProcessContent(content)
```

#### 성공 값 매핑

```par
catch e => .err e in
let try rawData = source.fetch in 
.ok Encode(rawData)
```

#### 실패 시 기본값 사용

```par
catch ! => "Unknown" in 
config.getUserName.try
```

## 레이블과 다층 오류 처리

`begin`/`loop`와 같이 `catch` 블록에도 레이블을 추가해 정밀한 제어가 가능하다.

```par
catch@label e => { ... }
```

대응하는 `try`/`throw` 명령에도 같은 레이블을 사용한다.

```par
let try@label value = result
throw@label "Custom error"
```

레이블은 오류 타입이 아니라 거리와 이름을 기준으로 선택된다. 구체적으로는 레이블(없는 경우도 포함하여)이 일치하는 가장 가까운 `catch` 블록이 선택된다. 이 방식으로 서로 다른 타입의 오류를 다른 핸들러로 처리할 수 있다.

```par
catch@fs e => { /* 파일시스템 오류 처리 */ }
catch@net e => { /* 네트워크 오류 처리 */ }

let try@fs writer = path.createFile
let try@net conn = url.connect
```

### 이전 `catch` 블록으로 오류 전파

또 다른 강력한 패턴으로는 중첩된 `catch` 블록으로 자원을 정리하고 공통 오류 처리를 바깥 블록에 위임하는 것이 있다.

이 패턴의 기본 사용 예제를 확인해 보자.

```par
catch e => {
  Debug.Log("Main error handler")
  Debug.Log(e)
  exit!
}

let try resource = AcquireResource
catch e => {
  resource.cleanup
  throw e  // 위의 메인 핸들러에 위임
}

// 자원을 사용하되 다른 곳에서도 오류가 발생할 수 있음
let try otherData = SomeOtherOperation  // 실패할 수도 있음
ProcessTogether(resource, otherData)
```

아래쪽의 `catch`에서는 특정한 `resource`의 정리를 담당하고, 공통 오류 출력 로직을 담당하는 위쪽 `catch`에 `throw`한다. `resource` 자체가 아니라 `SomeOtherOperation`에서 오류가 발생했을 때는 `resource`가 아직 유효하므로 제대로 정리해야 하기 때문이다.

위와 같은 패턴을 더 복잡하고 현실적인 상황 중 자원을 올바르게 관리하면서 파일을 복사하는 경우에 적용해 보자.

```par
def Main: ! = chan exit {
  let console = Console.Open

  catch ! => { console.print("Failed to read input.").close; exit! }
  console.prompt("Src path: ")[try src]
  console.prompt("Dst path: ")[try dst]

  catch e: Os.Error => {
    console.print("An error occurred:")
    console.print(e)
    console.close
    exit!
  }

  let try reader = Os.OpenFile(Os.Path(src))
  catch@w e => { reader.close; throw e }

  let try@w writer = Os.CreateOrReplaceFile(Os.Path(dst))
  catch@r e => { writer.close; throw e }

  reader.begin.read.try@r.case {
    .end! => {
      writer.close.try
      console.close
      exit!
    }
    .chunk(bytes) => {
      writer.write(bytes).try@w
      reader.loop
    }
  }
}
```

여기서 `catch@r`과 `catch@w` 블록에서는 자원별로 정리(파일 핸들 닫기)하는 로직을 담당하지만 그 뒤에는 오류를 출력하고 종료하는 공통 로직을 담당하는 메인 오류 핸들러로 `throw`한다.

이와 같은 계층적 접근으로 각 계층을 깔끔하게 유지하면서도 정교한 오류 처리 체계를 구축할 수 있다.

## 함수에서의 오류 전파

지금까지의 예제에서는 오류를 출력하고 종료하는 터미널 오류 처리만을 다루었지만, 함수 내에서 발생한 오류를 호출자에게 전파해야 하는 경우도 있다. 파일 전체의 내용을 읽는 다음 유틸리티 함수를 살펴 보자.

```par
module Main

import {
  @basic/Os
  @core/Bytes
  @core/Try
}

dec ReadAll : [Os.Path] Try<Os.Error, Bytes>
def ReadAll = [path]
  catch e => .err e in
  let try reader = Os.OpenFile(path) in
  Bytes.ReadAll(reader)
```

이 함수에서는 `Os.OpenFile(path)`로 파일을 연 뒤, `Bytes.ReadAll`로 분절된 `Bytes.Reader`를 하나의 `Bytes` 값으로 모아들인다. `catch` 블록에서는 발생하는 모든 오류를 `.err` 결과값으로 전달하는 한편, 성공할 경우 파일의 내용을 `.ok` 결과값으로 반환한다.

## `default`를 이용한 기본값 삽입

옵션 값이 비어 있을 때, 굳이 분기하지 않고 폴백 값으로 대체만 하고 싶은 경우도 있다. `default` 문법 설탕으로 바로 이 동작을 구현할 수 있다.

이 문법은 `try`/`catch`와는 별개이다. `try`는 `Try` 값을 풀어내어 `.err`를 전파하고, `default`는 `Option` 값을 풀어내어 `.none!`을 대체하는 문법이다. `Try` 값에서 오류를 무시하고 싶다면 우선 `Try.ToOption`으로 변환해야 한다.

- 후위 형태 (식/명령):

  ```par
  let r1: Option<Int> = .some 7
  let r2: Option<Int> = .none!

  let x = r1.default(0)   // x = 7
  let y = r2.default(0)   // y = 0

  let result: Try<String, Int> = .err "not a number"
  let option = Try.ToOption(result)
  let z = option.default(0)  // z = 0
  ```

  이 문법은 주어에 대한 `.case`로 변환된다. `.some`의 경우에는 풀어낸 값을 가지고 진행하고, `.none`의 경우에는 폴백 식을 평가한 뒤 그 값을 대신 사용한다. `.default`는 국소적 변환이므로 `let` 대입을 포함해 식을 사용할 수 있는 어디든 직접 사용할 수 있다.

- 패턴 형태 (수신 명령 포함):

  ```par
  let default(0) n = Nat.FromString("oops")
  ```

  이 패턴은 `.some`의 경우 그대로 대입하고, `.none`의 경우 폴백 식을 대입한다.

  패턴 형태가 수신 명령에서 특히 유용하다는 것을 보여주는 실용적인 예제를 보자. 맵을 사용하여 단어가 등장한 횟수를 세되, 특정 키가 없을 경우에는 `0`에서 시작한다.

  ```par
  dec Counts : [List<String>] List<(String) Nat>
  def Counts = [words] do {
    let counts = Map.New(type String, type Nat)
    words.begin.case {
      .end! => {}
      .item(word) => {
        counts.entry(word)[default(0) count]
        counts.put(count + 1)
        words.loop
      }
    }
  } in counts.list
  ```

  `.item` 분지에서는 `counts.entry(word)`를 통해 `Option<Nat>`을 수신받는다. 값이 없는 경우에는 `default(0)` 패턴에서 `count`에 `0`을 대입하여 자연스럽게 처리한다.
