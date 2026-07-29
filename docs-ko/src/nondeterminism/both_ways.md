# 양방향 통신

지금까지는 정보가 클라이언트에서 서버로 한 방향으로만 흘렀다. **반대 방향으로는 어떨까?**

이런 서버를 구현하려 한다고 생각해 보자.
- 클라이언트의 요청이 들어오면 고유한 ID 값으로 응답하는 서버
- 클라이언트끼리 선형 자원을 주고받는 과정을 관리하는 서버
- 클라이언트가 서로에게 보내는 메시지를 전달하는 서버

위의 세 기능 모두 클라이언트에서 서버로뿐만이 아니라 **서버에서 클라이언트로** 정보가 흐르는 것을 구현해야 한다. 물론 **모두 무리 없이 가능하다!**

이 기능으로 지원하지 **않는** 것을 짚고 넘어가자.
- **서버가 상호작용을 시작하는 동작**: `poll`/`submit` 문법에서는 클라이언트만이 준비 상태가 됨으로써 상호작용을 시작한다.
- **비결정론적 통신 방향**: [이번 단원의 도입부](./index.md)에서 이미 설명했듯이, Par는 아직은 이 기능을 지원하지 **못한다**. 타입을 보면 항상 어느 쪽이 통신할 차례인지를 알 수 있다.

## [소멸에 의한 생성](../processes/duality.md) 복습

초반에는 식 문법만을 다루는 것으로 시작했지만 가끔 [프로세스 문법](../process_syntax.md)으로 전환하는 것이 도움이 된다는 것을 배운 바가 있다. 특히 입출력 연산과 리스트나 트리 따위의 자료구조를 비동기로 생성하는 연산을 결합할 때가 이에 해당한다.

이전 장에서 다룬 `ListFan<a>` 타입을 살펴 보자.

```par
type ListFan<a> = recursive either {
  .end!,
  .spawn(self) self,
  .item(a) self,
}
```

기존에는 `List<List<a>>`를 합친 결과로 사용했지만, 이 타입 자체를 단독으로 사용하지 말라는 법은 없다.

```par
dec ServeListFan : [<a> ListFan<a>] List<a>
def ServeListFan = ...
```

코드 자체는 [이전 장](./fan_pattern.md#팬-변환)의 `MergeLists`를 거의 그대로 사용할 수 있다.

이 함수를 어떻게 사용하면 될까? 식 문법으로는 자명한 방법이 있다.

```par
ServeListFan(
  .spawn(.item(1).item(2).end!)
  .spawn(.item(3).item(4).end!)
  .end!
)
```

문제 없이 작동하지만, 이 코드를 입출력 연산과 결합하려면 [프로세스 문법](../process_syntax.md)을 사용하는 것이 더 좋을 것이다. 여기서는 [`chan`](../processes/chan_expression.md)식을 사용해 `ListFan<a>`의 사용자를 소멸시켜서 값을 생성해 보자.

```par
ServeListFan(chan server {
  server.spawn(chan server {
    // 아무 데서나 입출력 가능
    server.item(1)
    server.item(2)
    server.end!
  })
  server.spawn(chan server {
    // 아무 데서나 입출력 가능
    server.item(3)
    server.item(4)
    server.end!
  })
  server.end!
})
```

명령형 스타일의 코드가 되었다! `.item`과 `.end` 호출 사이에 원하는 종류의 입출력 연산을 삽입하는 것을 생각하는 것은 어렵지 않을 것이다.

이때 `server`의 타입은 무엇인가? 바로 `ListFan<a>`의 쌍대이다.

```par
iterative choice {
  .end => ?,
  .spawn(dual self) => self,
  .item(a) => self,
}
```

**위의 타입은 *클라이언트*의 입장에서 *서버*를 본 것에 해당한다.**

[쌍대성](../processes/duality.md)에서 배웠던 것을 잠시 복습해 보자.
- 후속문 타입 `?`은 모든 자원을 사용한 뒤 완료해야 한다는 의무에 해당한다.
  
  > **서버-클라이언트 구조에서는 반드시 클라이언트가 서버보다 먼저 완료해야 한다.** 이는 Par의 동시성 모델의 기본 원리에서 따라온다. 프로세스끼리 채널로 연결되어 하나의 트리를 이루기 때문에 교착 상태나 프로세스 누수가 발생하지 않는다.
- `.spawn`의 결과 타입인 `(dual self) => self`는 단순히 위에서 봤던 이 타입의 쌍대에 해당한다.
  
  ```par
  .spawn(self) self
  ```
  
  이 타입은 서버의 시점에서 본 것이지만, `dual ListFan<a>`는 클라이언트의 시점에 해당한다.

이제 양방향으로 통신하는 방법을 알아 보자!

## 예제: 고유 ID 부여

클라이언트 몇 개를 생성한 뒤 각각에 고유한 ID를 부여한다고 생각해 보자. 클라이언트의 시점에서는 다음과 같은 인터페이스를 구상할 수 있다.

```par
type IdServer = iterative choice {
  .end => ?,
  .spawn(dual self) => self,
  .getId => (Nat) self,
}
```

> 클라이언트는 반드시 [재귀](../types/recursive.md)여야 하므로, 클라이언트의 시점에서는 서버가 [반복](../types/iterative.md) 타입이 된다.

서버의 시점에서 본 쌍대 타입은 다음과 같다.

```par
//            = dual IdServer
type IdClient = recursive either {
  .end!,
  .spawn(self) self,
  .getId [Nat] self,
}
```

위에서 본 것과 매우 비슷하지만, `.getId` 선지에서는 값을 주는 것이 아니라 받고 있는 것을 볼 수 있다.

이 프로토콜을 구현하는 서버는 다음과 같다. 주석을 함께 읽어 보자.

```par
module Main

import @core/Nat

dec ServeIds : [IdClient] !
def ServeIds = [clients] do {
  // 서버 내부 상태 초기화
  // 여기서는 매번 증가하는 `id` 변수 하나로만
  // 이루어져 있다.
  let id = 0
} in poll(clients) {
  client => client.case {
    // 팬 구조 처리
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    // 클라이언트에서 ID를 요청함
    .getId client => do {
      // 제자리에서 `id` 증가
      id += 1
      // 여기서 `client`의 타입은 `[Nat] IdClient`
      // 송신 명령으로 새로운 ID를 전송
      client(id)
    } in submit(client),
  }
  else => !
}
```

> 매번 `submit`을 할 때마다 `id` 변수 역시 암시적으로 전달되는 것을 확인할 수 있다. `poll`/`submit`에서 지역 변수의 취급은 `.begin`/`.loop`와 같으며, 항상 최신의 값을 유지한다.

사용할 때는 이렇게 할 수 있다.

```par
module Main

import {
  @core/Debug
  @core/Nat
  @core/String
}

def SimpleClient: [String] IdClient = [name]
  // `IdServer`와 `IdClient`가 서로 쌍대이므로
  // `IdServer`를 소멸시켜서 `IdClient`를 생성할 수 있음
  chan server: IdServer {
    server.getId[id]
    Debug.Log(`${name} got number #{id}`)
    server.end!
  }

def Main: ! = ServeIds(chan server {
  server.spawn(SimpleClient("A"))
  server.spawn(SimpleClient("B"))
  server.spawn(SimpleClient("C"))
  server.spawn(SimpleClient("D"))
  server.end!
})
```

위의 프로그램을 그대로 실행하면 거의 무조건 클라이언트 A, B, C, D에 1, 2, 3, 4가 부여될 것이다. 이는 단순히 클라이언트가 생성되자마자 ID를 요청하기 때문으로, 그 이전에 다른 작업을 했거나 ID를 여러 개 요청했다면 결과가 매번 달랐을 것이다.

요점은 모든 클라이언트가 독립적으로 실행되며, 어느 시점에든 ID를 요청하면 매번 새로운 ID를 받을 수 있다는 점이다.

## 예제: 자원 공유

서버는 동기적으로 실행된다는 점을 이용해 클라이언트에게 공유 자원에 단독 접근 권한을 부여할 수 있다.

1. 초기에는 서버가 자원을 가지고 있다.
2. 클라이언트가 자원을 요청하면 서버가 자원을 전송한다.
3. 서버에서 부여한 클라이언트의 단독 세션은 클라이언트가 자원을 돌려줄 때까지 지속된다.
4. 클라이언트가 자원을 돌려주면 다른 클라이언트가 자원을 받을 수 있다.
5. 모든 클라이언트가 종료하면 자원은 서버의 생성자에게 돌아간다.

클라이언트의 입장에서는 다음과 같은 인터페이스를 설계할 수 있다.

```par
type MutexServer<a> = iterative choice {
  .end => ?,
  .spawn(dual self) => self,
  .take => (a) choice {
    .put(a) => self,
  }
}
```

`.take` 메서드를 자세히 살펴 보자.
- `.take => (a) choice {`
  
  자원을 요청하면 `a` 값을 직접 받을 수 있지만, 그 뒤에는 서버 자체가 메서드가 하나뿐인 새로운 선택 값으로 변환된다.
- `.put(a) => self,`
  
  이 메서드 `.put`에서는 위에서 받은 `a` 값을 요구한다. 자원을 돌려놓으면 서버가 원래 프로토콜로 돌아간다.

서버와 클라이언트 사이의 단독 세션은 클라이언트가 자원을 돌려줄 때까지 지속된다.

서버를 구현하려면 모든 클라이언트가 완료한 뒤 남은 값을 어떻게 할지 결정해야 한다. 여기서는 서버 생성자에게 값을 돌려준다.

```par
type MutexClient<a> = dual MutexServer<a>

dec ShareMutex : [<a> a, MutexClient<a>] a
def ShareMutex = [<a> value, clients] poll(clients) {
  client => client.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    .take session => session(value).case {
      .put(value) client => submit(client),
    }
  }

  else => value,
}
```

이 부분에 집중해 보자.

```par
    .take session => session(value).case {
      .put(value) client => submit(client),
    }
```

클라이언트가 `.take`를 선택하면 다음 함수가 되어 `session`에 대입된다.

```par
[a] either {
  .put(a) MutexClient<a>,
}
```

이 함수에 `value`를 전달한 뒤, `.case`를 사용해 클라이언트에서 값을 돌려줄 때까지 기다린다. 그 뒤에는 해당 클라이언트를 다시 풀에 제출한다.

실제 사용은 다음과 같이 할 수 있다.

```par
dec IncrementingClient : [Nat] MutexClient<Nat>
def IncrementingClient = [count] chan server {
  Nat.Repeat(count).begin.case {
    .step rest => {
      server.take[n]
      n += 1
      server.put(n)
      rest.loop
    }
    .end! => {
      server.end!
    }
  }
}

def Main: ! = do {
  let final = ShareMutex(0, chan server {
    server.spawn(IncrementingClient(1000))
    server.spawn(IncrementingClient(1000))
    server.spawn(IncrementingClient(1000))
    server.end!
  })
  Debug.Log(`#{final}`)
} in !
```

위에서는 클라이언트 3개를 생성하며, 각 클라이언트는 값을 획득해 1을 더한 뒤 돌려주는 과정을 1000회씩 수행한다. 작업이 끝나면 3000이 출력된다.

> 💡 **이론이 궁금하다면?** 위의 `MutexServer`로 [**_Safe session-based concurrency with shared linear state_**](https://library.oapen.org/bitstream/handle/20.500.12657/63011/1/978-3-031-30044-8.pdf#page=434) 논문을 커버하고 있다.
>
> 세 논문 중 `poll`/`submit`으로 대체할 수 있는 남은 하나인 [**_Concurrency and races in classical linear logic_**](https://cs.au.dk/~birke/phd-students/QianZ-thesis.pdf)에서는 다음과 같은 형태의 서버를 집중적으로 다룬다.
>
> ```par
> type Server = iterative choice {
>   .end => ?,
>   .spawn(dual self) => self,
> 
>   .method1(A1) => (B1) self,
>   .method2(A2) => (B2) self,
>   ...
> }
> ```
>
> 물론 이 역시 `poll`/`submit`으로 커버할 수 있다!

## 예제: [플레이그라운드](../getting_started.md)에서 채팅 서버 구현

서버와 클라이언트 사이의 양방향 통신을 사용하는 심화 예제는 [Par의 대화형 플레이그라운드](../getting_started.md)에서 직접 사용해볼 수 있는 토이 채팅 서버이다.

서버를 통해 클라이언트가 서로 통신하는 과정을 확인할 수 있다.

프로젝트의 [`examples`](https://github.com/par-team/par-lang/blob/main/examples/src/PlaygroundChat.par) 디렉토리에서 꼼꼼하게 주석을 남긴 토이 채팅 서버의 코드를 확인할 수 있다.
