# `repoll`을 사용한 동작 전환

간혹 서버에서 '기어를 바꿔야 하는' 경우도 있다.

여러 개의 취소 가능한 스트림을 하나의 스트림으로 합치는 경우를 생각해 보자.
- 합친 스트림을 사용 중일 때는 소스에서 원소를 계속 가지고 와야 한다.
- 하지만 사용자가 합친 스트림을 취소했을 때는 **서버에서 관리하는 모든 스트림 역시 취소해야 한다.**

`repoll`로 바로 이런 경우를 해결할 수 있다. `repoll`은 같은 풀을 재사용하는 대신, *다른 핸들러*를 사용해 폴링하기 시작한다.

## 취소 가능한 스트림, `Source<a>`

스트림을 취소하는 동작을 `List<a>`와 비교해 보자.

`List<a>`는 한 번 생성하면 리스트가 끝날 때까지 계속 원소를 생성하며, 사용자가 중간에 멈추려고 해도 멈출 수 없다. 스트림을 취소할 수 있으려면 생산자와 사용자가 협력하는 프로토콜로 전환해야 한다. 생산자도 작업을 *제공*할 수 있지만, 사용자도 멈출 것을 선택할 수 있다.

스트림과 비슷한 이 프로토콜을 `Source<a>`라고 하자. 처음에는 다음과 같은 타입을 생각해 볼 수 있다.

```par
type Source<a> = recursive choice {
  .close => !,
  .next => either {
    .end!,
    .item(a) self,
  }
}
```

이 프로토콜 자체는 전혀 문제가 없지만, `poll`과는 상성이 좋지 않다. 소스는 서버에서 `.close`와 `.next` 중 하나를 고를 때까지 기다려야 하기 때문에 알아서 미리 '준비'할 수 없으며 동시성을 잃게 된다.

두 번째로는 일단 소스에서 원소를 생성한 다음에 사용자가 계속할지 결정할 수 있는 프로토콜을 생각할 수 있다.

```par
type Source<a> = recursive either {
  .end!,
  .item(a) choice {
    .close => !,
    .next => self,
  }
}
```

처음보다는 낫지만, 여러 소스를 합치는 것은 다소 어려워진다. 합친 스트림의 사용자가 스트림을 `.close`한다면, 하위 소스에서 이미 `a`를 더 생성했는데 보낼 곳이 없어진 상태가 될 수도 있다.

원소를 생성했다는 신호와 원소를 전달하라는 신호를 분리하면 프로토콜을 올바르게 설계할 수 있다.

```par
type Source<a> = recursive either {
  .end!,
  .item choice {
    .discard => !,
    .get => (a) self,
  }
}
```

여기서는 소스가 알아서 코드를 실행(하여 `.item`이나 `.end!`를 생성)할 수 있으므로 `poll`과 상성이 좋다. 하지만 `.item`을 생성하더라도 그 값을 즉시 전달하지는 않으며, 사용자에게 최종 결정을 맡긴다.
- `.get`을 선택하면 사용자에게 값을 전달하고 계속한다.
- `.discard`를 선택하면 취소하고 `!`로 마무리한다.

> 💡 이 구조를 **협력적 취소**라 한다. 생산자가 취소를 받아들일 수 있는 상태(여기서는 `.item`을 생성한 직후)에만 사용자가 실제로 취소할 수 있다.

## 생산 모드에서 취소 모드로 전환

이제 여러 소스를 하나로 합친다고 생각해 보자.

```par
dec MergeSources : [<a> List<Source<a>>] Source<a>
```

합친 소스를 취소했을 때는 하위 소스 역시 모두 취소해야 한다. 이제 서버에서는 두 가지 서로 다른 모드를 지원해야 한다.

- **생산 모드**: 준비된 소스에서 원소를 가져와서 사용자에게 넘긴다.
- **취소 모드**: 원소를 가져오는 동작을 멈추고 남은 소스를 모두 버린다.

`repoll`로써 이와 같은 모드 전환을 표현할 수 있다.

## `repoll`

`repoll`은 `poll`과 비슷하지만, 자체적으로 새로운 풀을 생성하는 대신 가장 가까운 상위 `poll`(있을 경우)의 풀을 재사용하고, 선택적으로 클라이언트를 더 추가한 뒤 새로운 핸들러를 가지고 폴링을 시작한다.

```par
poll(...) {
  client => ... repoll(...) {
    client => ...
    else => ...
  }

  else => ...
}
```

`repoll`은 **반드시** `poll`이나 다른 `repoll`의 활성 조건지 *안*에서 사용해야 한다.

## 예제: 취소 가능한 소스 합치기

우선 [팬 패턴](./fan_pattern.md)에서 알아보았던 팬 구조를 `Source<a>`에 적용할 것이다.

```par
type Source<a> = recursive either {
  .end!,
  .item choice {
    .discard => !,
    .get => (a) self,
  }
}

type SourceFan<a> = recursive either {
  .end!,
  .spawn(self) self,
  .item choice {
    .discard => !,
    .get => (a) self,
  }
}

dec SourceFan : [<a> List<Source<a>>] SourceFan<a>
def SourceFan = [<a> sources] sources.begin.case {
  .end! => .end!,
  .item(source) sources => .spawn(source) sources.loop,
}
```

이제 이 함수를 구현할 수 있다.

```par
dec MergeSources : [<a> List<Source<a>>] Source<a>
```

`MergeSources`를 구현할 때 두 가지 모드를 사용하는 것이 핵심이다.
- **생산 모드**: 소스를 폴링하면서 원소를 계속 생산한다.
- **취소 모드**: 사용자가 취소하면 그 즉시 풀에 남은 모든 소스를 취소한다.

전체적인 구조는 다음과 같다.

```par
def MergeSources = [<a> sources] poll(SourceFan(sources)) {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    .item s => .item case {
      .discard => ...   // 취소 모드로 전환
      .get => ...       // 원소 1개를 생산하고 계속 진행
    }
  }

  else => .end!,
}
```

### 생산 모드: 원소 생산

`.item` 분지에서는 현재 폴링한 소스에서 원소를 생성했고 사용자의 결정을 기다리고 있다.

사용자가 `.get`을 선택하면 해당 소스에서 값을 요청해 반환한 뒤 소스를 풀로 되돌린다.

```par
.get => do { s.get[x] } in (x) submit(s),
```

왼쪽에서 오른쪽으로 읽으면 된다.
- `s.get[x]`로 현재 소스에서 `a` 값을 요청한다.
- `(x)`로 합쳐진 `Source<a>`에서 값을 반환한다.
- `submit(s)`로 소스를 다른 값을 계속 생성할 수 있도록 풀로 되돌린다.

### 취소 모드: 남은 자원을 취소

사용자가 `.discard`를 선택하면 다음과 같이 남은 자원을 모두 취소해야 한다.
1. 현재 소스 `s` (지금 코드상에서 가지고 있다)
2. 풀에 남아 있는 나머지 모든 소스

현재 소스는 바로 버릴 수 있다.

```par
let ! = s.discard in ...
```

그런 다음에는 (추가 클라이언트 없이) `repoll()`을 사용해 **기어를 전환**하여, 같은 풀을 재사용하되 핸들러를 바꾸어 풀을 비운다.

```par
.discard => let ! = s.discard in repoll() {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),
    .item s => let ! = s.discard in submit(),
  }
  else => !,
}
```

`repoll()`에서는 아무런 `a` 값도 생산하지 않고, 풀이 빌 때까지 소스를 꺼내어 버린 뒤 `!`를 반환한다.

## 전체 코드

코드를 완성하면 다음과 같다.

```par
module Main

import @core/List

dec MergeSources : [<a> List<Source<a>>] Source<a>
def MergeSources = [<a> sources] poll(SourceFan(sources)) {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    .item s => .item case {
      .discard => let ! = s.discard in repoll() {
        fan => fan.case {
          .end! => submit(),
          .spawn(l) r => submit(l, r),
          .item s => let ! = s.discard in submit(),
        }
        else => !,
      }

      .get => do { s.get[x] } in (x) submit(s),
    }
  }

  else => .end!,
}
```

> **`poll` / `repoll` / `submit`의 레이블**: `poll@label(...)`, `repoll@label(...)`, `submit@label(...)`과 같이 추가로 레이블을 달 수도 있다.
>
> 레이블은 범위 안에 (`repoll`로 생성된) 여러 개의 폴링 지점이 있을 때 `submit`이 *어떤 폴링 지점*으로 점프할지 지정하는 역할을 한다.
>
> 레이블이 없는 것도 레이블로 취급된다. `submit(...)`이라고 작성하면 레이블이 없는 가장 가까운 폴링 지점을, `submit@x(...)`라고 작성하면 `@x` 레이블이 있는 폴링 지점을 선택한다.
